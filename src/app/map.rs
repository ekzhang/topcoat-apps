#![allow(clippy::too_many_arguments)]

use std::{
    error::Error as StdError,
    fmt,
    num::NonZeroUsize,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::head;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use fast_mvt::MvtReaderRef;
use lru::LruCache;
use tokio::{
    sync::{OnceCell, Semaphore},
    task::JoinSet,
};
use topcoat::{
    Result,
    router::page,
    runtime::{Event, shard},
    view::view,
};

mod render;

const LOGICAL_WIDTH: u32 = 1_000;
const LOGICAL_HEIGHT: u32 = 600;
const DEVICE_PIXEL_RATIO: f64 = 2.0;
const FRAME_WIDTH: u32 = LOGICAL_WIDTH * 2;
const FRAME_HEIGHT: u32 = LOGICAL_HEIGHT * 2;
const TILE_SIZE: f64 = 256.0;
const MIN_ZOOM: f64 = 2.0;
const MAX_ZOOM: f64 = 17.0;
const MAX_SOURCE_ZOOM: u8 = 14;
const MAX_LATITUDE: f64 = 85.051_128_78;
const CITY_VIEWPORTS: [CityViewport; 11] = [
    CityViewport::new(9.9325, -84.08, 12.0),
    CityViewport::new(35.6812, 139.7671, 11.5),
    CityViewport::new(48.8566, 2.3522, 12.0),
    CityViewport::new(40.7128, -74.006, 11.5),
    CityViewport::new(41.0082, 28.9784, 11.5),
    CityViewport::new(-22.9068, -43.1729, 11.5),
    CityViewport::new(-33.9249, 18.4241, 11.5),
    CityViewport::new(1.3521, 103.8198, 11.0),
    CityViewport::new(-33.8688, 151.2093, 11.5),
    CityViewport::new(19.4326, -99.1332, 11.5),
    CityViewport::new(22.3193, 114.1694, 11.5),
];
const TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";
const MAP_REFERER: &str = "https://topcoat-apps.modal.ekzhang.com/map";

static RENDER_SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
static TILE_CACHE: LazyLock<Mutex<LruCache<TileKey, Bytes>>> = LazyLock::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(256).expect("tile cache capacity is non-zero"),
    ))
});
static TILE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("topcoat-apps-map/0.1 (+https://github.com/ekzhang/topcoat-apps)")
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(12))
        .pool_idle_timeout(Duration::from_secs(300))
        .pool_max_idle_per_host(32)
        .tcp_keepalive(Duration::from_secs(60))
        .http2_adaptive_window(true)
        .http2_keep_alive_interval(Duration::from_secs(60))
        .http2_keep_alive_timeout(Duration::from_secs(5))
        .http2_keep_alive_while_idle(true)
        .build()
        .expect("map tile HTTP client should build")
});
#[derive(Clone, Copy)]
struct CityViewport {
    latitude: f64,
    longitude: f64,
    zoom: f64,
}

impl CityViewport {
    const fn new(latitude: f64, longitude: f64, zoom: f64) -> Self {
        Self {
            latitude,
            longitude,
            zoom,
        }
    }

    fn viewport(self) -> String {
        format!("{},{},{}", self.latitude, self.longitude, self.zoom)
    }
}

fn daily_viewport() -> String {
    let day = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    CITY_VIEWPORTS[day as usize % CITY_VIEWPORTS.len()].viewport()
}

fn city_viewports_json() -> String {
    let cities = CITY_VIEWPORTS
        .iter()
        .map(|city| {
            format!(
                r#"{{"latitude":{},"longitude":{},"zoom":{}}}"#,
                city.latitude, city.longitude, city.zoom
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{cities}]")
}

#[page]
async fn remote_map() -> Result {
    view! {
        head(
            title: "Static map generator · Topcoat apps",
            description: "An interactive slippy map rendered into raster frames by Rust on the server.",
            <link
                rel="stylesheet"
                href=(topcoat::asset::asset!("assets/map.css"))
            >
            <script
                src=(topcoat::asset::asset!("assets/map.js"))
                defer="defer"
            ></script>
        )
        <body>
            signal viewport = daily_viewport();
            signal loading = false;
            signal style_open = false;
            signal theme = "warm".to_string();
            signal roads = "standard".to_string();
            signal annotations = "standard".to_string();
            signal terrain = true;
            signal buildings = true;
            signal boundaries = true;

            <main
                class="mx-auto max-w-[1064px] px-5 py-8 sm:px-8 sm:py-12"
                @maprenderstart=$(|_event| loading.set(true))
                @maprendercomplete=$(|_event| loading.set(false))
            >
                <header class="mb-7">
                    <div>
                        <h1 class="text-3xl font-semibold tracking-[-0.045em] text-stone-950 sm:text-4xl">
                            "Static map generator"
                        </h1>
                        <p class="mt-2 max-w-2xl text-sm leading-6 text-stone-500">
                            "Drag or zoom the map. Server-side Rust styles OpenFreeMap vector tiles and paints a static JPEG frame using Skia. No map renderer runs in the browser."
                        </p>
                    </div>
                </header>

                <section class="overflow-hidden rounded-2xl border border-stone-200 bg-white shadow-sm">
                    <div class="flex flex-wrap items-center justify-between gap-3 border-b border-stone-200 px-4 py-3 sm:px-5">
                        <div>
                            <h2 class="text-sm font-semibold text-stone-950">"Map preview"</h2>
                        </div>
                        <div class="flex items-center gap-2">
                            <button
                                type="button"
                                @click=$(|_event| style_open.set(!style_open.get()))
                                :aria-expanded=$(style_open.get())
                                :aria-pressed=$(style_open.get())
                                class="h-8 rounded-lg border border-stone-200 bg-white px-3 text-xs font-semibold text-stone-700 transition hover:border-stone-300 hover:bg-stone-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-stone-400 aria-pressed:border-stone-900 aria-pressed:bg-stone-900 aria-pressed:text-white"
                            >
                                "Style"
                            </button>
                            <button
                                type="button"
                                data-map-action="download"
                                class="h-8 rounded-lg border border-stone-200 bg-white px-3 text-xs font-semibold text-stone-700 transition hover:border-stone-300 hover:bg-stone-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-stone-400"
                            >
                                "Screenshot"
                            </button>
                            <button
                                type="button"
                                data-map-action="random-city"
                                class="h-8 rounded-lg border border-stone-200 bg-white px-3 text-xs font-semibold text-stone-700 transition hover:border-stone-300 hover:bg-stone-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-stone-400"
                            >
                                "Random city"
                            </button>
                        </div>
                    </div>

                    <div
                        :hidden=$(!style_open.get())
                        class="border-b border-stone-200 bg-stone-50/80 px-4 py-4 sm:px-5"
                    >
                        <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-[1fr_1.25fr_1fr_1.15fr] lg:items-end">
                            <div>
                                <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-stone-400">
                                    "Palette"
                                </p>
                                <div
                                    role="group"
                                    aria-label="Map palette"
                                    class="inline-flex h-8 items-center rounded-lg bg-stone-200/80 p-1"
                                >
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            theme.set("warm".to_owned());
                                        })
                                        :disabled=$(theme.get() == "warm")
                                        :aria-pressed=$(theme.get() == "warm")
                                        class="h-6 rounded-md px-2.5 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Warm"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            theme.set("verdant".to_owned());
                                        })
                                        :disabled=$(theme.get() == "verdant")
                                        :aria-pressed=$(theme.get() == "verdant")
                                        class="h-6 rounded-md px-2.5 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Verdant"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            theme.set("night".to_owned());
                                        })
                                        :disabled=$(theme.get() == "night")
                                        :aria-pressed=$(theme.get() == "night")
                                        class="h-6 rounded-md px-2.5 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Night"
                                    </button>
                                </div>
                            </div>

                            <div>
                                <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-stone-400">
                                    "Roads"
                                </p>
                                <div
                                    role="group"
                                    aria-label="Road emphasis"
                                    class="inline-flex h-8 items-center rounded-lg bg-stone-200/80 p-1"
                                >
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            roads.set("off".to_owned());
                                        })
                                        :disabled=$(roads.get() == "off")
                                        :aria-pressed=$(roads.get() == "off")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Off"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            roads.set("quiet".to_owned());
                                        })
                                        :disabled=$(roads.get() == "quiet")
                                        :aria-pressed=$(roads.get() == "quiet")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Quiet"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            roads.set("standard".to_owned());
                                        })
                                        :disabled=$(roads.get() == "standard")
                                        :aria-pressed=$(roads.get() == "standard")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Normal"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            roads.set("bold".to_owned());
                                        })
                                        :disabled=$(roads.get() == "bold")
                                        :aria-pressed=$(roads.get() == "bold")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Bold"
                                    </button>
                                </div>
                            </div>

                            <div>
                                <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-stone-400">
                                    "Annotations"
                                </p>
                                <div
                                    role="group"
                                    aria-label="Annotation density"
                                    class="inline-flex h-8 items-center rounded-lg bg-stone-200/80 p-1"
                                >
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            annotations.set("off".to_owned());
                                        })
                                        :disabled=$(annotations.get() == "off")
                                        :aria-pressed=$(annotations.get() == "off")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Off"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            annotations.set("sparse".to_owned());
                                        })
                                        :disabled=$(annotations.get() == "sparse")
                                        :aria-pressed=$(annotations.get() == "sparse")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Light"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            annotations.set("standard".to_owned());
                                        })
                                        :disabled=$(annotations.get() == "standard")
                                        :aria-pressed=$(annotations.get() == "standard")
                                        class="h-6 rounded-md px-2 text-[11px] font-semibold text-stone-500 transition hover:text-stone-900 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                                    >
                                        "Full"
                                    </button>
                                </div>
                            </div>

                            <div>
                                <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-stone-400">
                                    "Layers"
                                </p>
                                <div class="flex h-8 items-center gap-1.5">
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            terrain.set(!terrain.get());
                                        })
                                        :aria-pressed=$(terrain.get())
                                        class="h-8 rounded-lg border border-stone-200 bg-white px-2.5 text-[11px] font-semibold text-stone-500 transition hover:border-stone-300 hover:text-stone-900 aria-pressed:border-stone-900 aria-pressed:bg-stone-900 aria-pressed:text-white"
                                    >
                                        "Terrain"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            buildings.set(!buildings.get());
                                        })
                                        :aria-pressed=$(buildings.get())
                                        class="h-8 rounded-lg border border-stone-200 bg-white px-2.5 text-[11px] font-semibold text-stone-500 transition hover:border-stone-300 hover:text-stone-900 aria-pressed:border-stone-900 aria-pressed:bg-stone-900 aria-pressed:text-white"
                                    >
                                        "Buildings"
                                    </button>
                                    <button
                                        type="button"
                                        @click=$(|_event| {
                                            loading.set(true);
                                            boundaries.set(!boundaries.get());
                                        })
                                        :aria-pressed=$(boundaries.get())
                                        class="h-8 rounded-lg border border-stone-200 bg-white px-2.5 text-[11px] font-semibold text-stone-500 transition hover:border-stone-300 hover:text-stone-900 aria-pressed:border-stone-900 aria-pressed:bg-stone-900 aria-pressed:text-white"
                                    >
                                        "Borders"
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>

                    <div
                        data-map-root=""
                        :data-map-viewport=$(viewport.get())
                        :data-map-theme=$(theme.get())
                        data-map-cities=(city_viewports_json())
                        :aria-busy=$(loading.get())
                        tabindex="0"
                        role="application"
                        aria-label="Server-rendered interactive map. Drag to pan and use the controls to zoom."
                        class="remote-map relative isolate aspect-[5/3] overflow-hidden bg-stone-200 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-stone-500"
                    >
                        <input
                            data-map-state=""
                            type="hidden"
                            :value=$(viewport.get())
                            @input=$(|event: Event| viewport.set(event.target.value))
                        >

                        map_frame(
                            viewport: $(viewport.get()),
                            theme: $(theme.get()),
                            roads: $(roads.get()),
                            annotations: $(annotations.get()),
                            terrain: $(terrain.get()),
                            buildings: $(buildings.get()),
                            boundaries: $(boundaries.get()),
                        )

                        <div
                            data-map-loading=""
                            :hidden=$(!loading.get())
                            class="pointer-events-none absolute inset-0 z-20 grid place-items-center bg-stone-950/10"
                        >
                            <span class="flex items-center gap-2 rounded-full bg-white/95 px-3 py-2 text-xs font-semibold text-stone-800 shadow-sm backdrop-blur">
                                <span class="size-3.5 animate-spin rounded-full border-2 border-stone-300 border-t-stone-900"></span>
                                "Rendering"
                            </span>
                        </div>

                        <div data-map-control="" class="absolute right-3 top-3 z-30 grid overflow-hidden rounded-lg border border-stone-300/80 bg-white/95 shadow-sm backdrop-blur">
                            <button
                                type="button"
                                data-map-action="zoom-in"
                                aria-label="Zoom in"
                                class="grid size-9 place-items-center border-b border-stone-200 text-lg font-medium text-stone-800 transition hover:bg-stone-100"
                            >
                                "+"
                            </button>
                            <button
                                type="button"
                                data-map-action="zoom-out"
                                aria-label="Zoom out"
                                class="grid size-9 place-items-center text-lg font-medium text-stone-800 transition hover:bg-stone-100"
                            >
                                "−"
                            </button>
                        </div>
                    </div>
                </section>

                <section class="mt-5 grid gap-px overflow-hidden rounded-xl border border-stone-200 bg-stone-200 sm:grid-cols-3">
                    <div class="bg-white p-4">
                        <p class="text-[11px] font-semibold uppercase tracking-[0.12em] text-stone-400">"Interaction"</p>
                        <p class="mt-1.5 text-sm leading-5 text-stone-700">
                            "Panning is previewed locally, then committed once per gesture."
                        </p>
                    </div>
                    <div class="bg-white p-4">
                        <p class="text-[11px] font-semibold uppercase tracking-[0.12em] text-stone-400">"Rust pipeline"</p>
                        <p class="mt-1.5 text-sm leading-5 text-stone-700">
                            "MVT geometry is styled, rasterized at 2×, and JPEG-encoded."
                        </p>
                    </div>
                    <div class="bg-white p-4">
                        <p class="text-[11px] font-semibold uppercase tracking-[0.12em] text-stone-400">"Transport"</p>
                        <p class="mt-1.5 text-sm leading-5 text-stone-700">
                            "A Topcoat shard swaps the complete raster frame and its metadata."
                        </p>
                    </div>
                </section>
            </main>
        </body>
    }
}

#[shard]
async fn map_frame(
    viewport: String,
    theme: String,
    roads: String,
    annotations: String,
    terrain: bool,
    buildings: bool,
    boundaries: bool,
) -> Result {
    let viewport = Viewport::parse(&viewport);
    let style =
        render::MapStyle::parse(&theme, &roads, &annotations, terrain, buildings, boundaries);
    let _permit = RENDER_SLOTS.clone().acquire_owned().await?;
    let rendered = render_map(viewport, style).await;

    match rendered {
        Ok(frame) => {
            let source = format!("data:image/jpeg;base64,{}", BASE64.encode(&frame.jpeg));
            let center = format!(
                "{} · {}",
                format_coordinate(frame.viewport.latitude, "N", "S"),
                format_coordinate(frame.viewport.longitude, "E", "W"),
            );
            let tile_summary = if frame.fetched_tiles == 0 {
                format!("{} tiles cached", frame.cached_tiles)
            } else {
                format!(
                    "{} cached · {} fetched",
                    frame.cached_tiles, frame.fetched_tiles
                )
            };
            let render_time = format!("{:.0} ms", frame.elapsed.as_secs_f64() * 1_000.0);
            let image_size = format!("{:.0} KB", frame.jpeg.len() as f64 / 1_024.0);
            let zoom = format_zoom(frame.viewport.zoom);
            let alt = format!(
                "Server-rendered vector map centered at {}, zoom {}",
                center, zoom
            );

            view! {
                <img
                    data-map-frame-image=""
                    src=(source)
                    alt=(alt)
                    draggable="false"
                    class="absolute inset-0 size-full select-none object-cover"
                    onload="this.dispatchEvent(new Event('maprendercomplete', { bubbles: true }))"
                >

                <div class="pointer-events-none absolute bottom-3 left-3 z-10 flex max-w-[calc(100%-8rem)] flex-wrap gap-1.5 text-[10px] font-medium tabular-nums text-stone-700 sm:text-[11px]">
                    <span class="rounded-md bg-white/92 px-2 py-1 shadow-sm backdrop-blur">
                        (center)
                        " · z"
                        (zoom)
                    </span>
                    <span class="rounded-md bg-white/92 px-2 py-1 shadow-sm backdrop-blur">
                        (render_time)
                        " · "
                        (tile_summary)
                        " · "
                        (frame.feature_count)
                        " features · "
                        (image_size)
                    </span>
                </div>

                <span
                    data-map-control=""
                    class="absolute bottom-2 right-2 z-30 rounded bg-white/90 px-1.5 py-1 text-[9px] font-medium text-stone-600 backdrop-blur transition hover:bg-white hover:text-stone-950 sm:text-[10px]"
                >
                    <a href="https://openfreemap.org/" target="_blank">"OpenFreeMap"</a>
                    " © "
                    <a href="https://www.openmaptiles.org/" target="_blank">"OpenMapTiles"</a>
                    " · Data from "
                    <a href="https://www.openstreetmap.org/copyright" target="_blank">"OpenStreetMap"</a>
                </span>
            }
        }
        Err(error) => {
            view! {
                <div class="absolute inset-0 grid place-items-center bg-stone-100 px-6 text-center">
                    <div class="max-w-sm">
                        <p class="text-sm font-semibold text-stone-900">"The map frame could not be rendered."</p>
                        <p class="mt-2 text-xs leading-5 text-stone-500">(error.to_string())</p>
                    </div>
                </div>
                <img
                    src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw=="
                    alt=""
                    aria-hidden="true"
                    class="hidden"
                    onload="this.dispatchEvent(new Event('maprendercomplete', { bubbles: true }))"
                >
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Viewport {
    latitude: f64,
    longitude: f64,
    zoom: f64,
}

impl Viewport {
    fn parse(value: &str) -> Self {
        let mut parts = value.split(',');
        let latitude = parts.next().and_then(|part| part.parse().ok());
        let longitude = parts.next().and_then(|part| part.parse().ok());
        let zoom = parts.next().and_then(|part| part.parse().ok());

        let mut viewport = match (latitude, longitude, zoom, parts.next()) {
            (Some(latitude), Some(longitude), Some(zoom), None) => Self {
                latitude,
                longitude,
                zoom,
            },
            _ => Self::default(),
        };

        if !viewport.latitude.is_finite() || !viewport.longitude.is_finite() {
            return Self::default();
        }

        viewport.latitude = viewport.latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE);
        viewport.longitude = wrap_longitude(viewport.longitude);
        viewport.zoom = viewport.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        viewport
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            latitude: 9.9325,
            longitude: -84.08,
            zoom: 12.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TileKey {
    zoom: u8,
    x: u32,
    y: u32,
}

#[derive(Clone, Debug)]
struct TilePlacement {
    key: TileKey,
    raw_x: i32,
}

struct RenderTile {
    placement: TilePlacement,
    bytes: Bytes,
}

struct RenderedMap {
    viewport: Viewport,
    jpeg: Vec<u8>,
    elapsed: Duration,
    cached_tiles: usize,
    fetched_tiles: usize,
    feature_count: usize,
}

async fn render_map(viewport: Viewport, style: render::MapStyle) -> MapResult<RenderedMap> {
    let started = Instant::now();
    let placements = tile_placements(viewport);
    let mut cached_tiles = 0;
    let mut ready = Vec::with_capacity(placements.len());
    let mut missing = Vec::new();

    {
        let mut cache = TILE_CACHE
            .lock()
            .expect("map tile cache should not be poisoned");
        for placement in placements {
            if let Some(bytes) = cache.get(&placement.key).cloned() {
                cached_tiles += 1;
                ready.push(RenderTile { placement, bytes });
            } else {
                missing.push(placement);
            }
        }
    }

    let mut requests = JoinSet::new();
    for placement in missing {
        requests.spawn(async move {
            fetch_tile(placement.key)
                .await
                .map(|bytes| RenderTile { placement, bytes })
        });
    }
    let mut fetched = Vec::with_capacity(requests.len());
    while let Some(result) = requests.join_next().await {
        fetched.push(
            result
                .map_err(|error| MapError::new(format!("tile request task failed: {error}")))??,
        );
    }
    let fetched_tiles = fetched.len();

    {
        let mut cache = TILE_CACHE
            .lock()
            .expect("map tile cache should not be poisoned");
        for tile in &fetched {
            cache.put(tile.placement.key, tile.bytes.clone());
        }
    }
    ready.extend(fetched);
    ready.sort_by_key(|tile| (tile.placement.key.y, tile.placement.raw_x));

    tokio::task::spawn_blocking(move || {
        let feature_count = count_features(&ready)?;
        let jpeg = render::render_jpeg(viewport, style, &ready)?;

        Ok(RenderedMap {
            viewport,
            jpeg,
            elapsed: started.elapsed(),
            cached_tiles,
            fetched_tiles,
            feature_count,
        })
    })
    .await
    .map_err(|error| MapError::new(format!("map rendering task failed: {error}")))?
}

fn count_features(tiles: &[RenderTile]) -> MapResult<usize> {
    let mut count = 0;
    for tile in tiles {
        if tile.bytes.is_empty() {
            continue;
        }
        let reader = MvtReaderRef::new(&tile.bytes)
            .map_err(|error| MapError::new(format!("invalid vector tile: {error}")))?;
        count += reader
            .layers()
            .map(|layer| layer.feature_count())
            .sum::<usize>();
    }
    Ok(count)
}

fn tile_placements(viewport: Viewport) -> Vec<TilePlacement> {
    let source_zoom = source_zoom(viewport);
    let (center_x, center_y) = mercator_tile_position(viewport, source_zoom);
    let overzoom = 2_f64.powf(viewport.zoom - f64::from(source_zoom));
    let half_width = f64::from(LOGICAL_WIDTH) / (2.0 * TILE_SIZE * overzoom);
    let half_height = f64::from(LOGICAL_HEIGHT) / (2.0 * TILE_SIZE * overzoom);
    let first_x = (center_x - half_width).floor() as i32;
    let last_x = (center_x + half_width).floor() as i32;
    let tile_count = 2_i32.pow(u32::from(source_zoom));
    let first_y = ((center_y - half_height).floor() as i32).max(0);
    let last_y = ((center_y + half_height).floor() as i32).min(tile_count - 1);

    (first_x..=last_x)
        .flat_map(|raw_x| {
            (first_y..=last_y).map(move |raw_y| TilePlacement {
                key: TileKey {
                    zoom: source_zoom,
                    x: raw_x.rem_euclid(tile_count) as u32,
                    y: raw_y as u32,
                },
                raw_x,
            })
        })
        .collect()
}

fn source_zoom(viewport: Viewport) -> u8 {
    viewport
        .zoom
        .floor()
        .clamp(MIN_ZOOM, f64::from(MAX_SOURCE_ZOOM)) as u8
}

fn mercator_tile_position(viewport: Viewport, zoom: u8) -> (f64, f64) {
    let tile_count = 2_f64.powi(i32::from(zoom));
    let x = (viewport.longitude + 180.0) / 360.0 * tile_count;
    let latitude_radians = viewport.latitude.to_radians();
    let y = (0.5
        - ((1.0 + latitude_radians.sin()) / (1.0 - latitude_radians.sin())).ln()
            / (4.0 * std::f64::consts::PI))
        * tile_count;
    (x, y)
}

static TILE_TEMPLATE: OnceCell<String> = OnceCell::const_new();

async fn tile_template() -> MapResult<&'static str> {
    TILE_TEMPLATE
        .get_or_try_init(load_tile_template)
        .await
        .map(String::as_str)
}

async fn load_tile_template() -> MapResult<String> {
    let response = TILE_CLIENT
        .get(TILEJSON_URL)
        .header("Referer", MAP_REFERER)
        .send()
        .await
        .map_err(|error| MapError::new(format!("tile metadata request failed: {error}")))?
        .error_for_status()
        .map_err(|error| MapError::new(format!("tile metadata request failed: {error}")))?;
    let metadata: serde_json::Value = response
        .json()
        .await
        .map_err(|error| MapError::new(format!("tile metadata response failed: {error}")))?;
    metadata
        .get("tiles")
        .and_then(serde_json::Value::as_array)
        .and_then(|tiles| tiles.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| MapError::new("tile metadata did not contain a tile URL"))
}

async fn fetch_tile(key: TileKey) -> MapResult<Bytes> {
    let template = tile_template().await?;
    let url = template
        .replace("{z}", &key.zoom.to_string())
        .replace("{x}", &key.x.to_string())
        .replace("{y}", &key.y.to_string());
    let response = TILE_CLIENT
        .get(&url)
        .header("Referer", MAP_REFERER)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| MapError::new(format!("tile request failed: {error}")))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| MapError::new(format!("tile response failed: {error}")))?;

    if bytes.len() > 8 * 1_024 * 1_024 {
        return Err(MapError::new("vector tile response exceeded 8 MB"));
    }

    Ok(bytes)
}

fn wrap_longitude(longitude: f64) -> f64 {
    (longitude + 180.0).rem_euclid(360.0) - 180.0
}

fn format_coordinate(value: f64, positive: &str, negative: &str) -> String {
    let direction = if value >= 0.0 { positive } else { negative };
    format!("{:.4}°{direction}", value.abs())
}

fn format_zoom(zoom: f64) -> String {
    let formatted = format!("{zoom:.2}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

type MapResult<T> = std::result::Result<T, MapError>;

#[derive(Debug)]
struct MapError {
    message: String,
}

impl MapError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for MapError {}
