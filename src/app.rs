mod fft;
mod map;

use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    font::fontsource::fontsource_font,
    router::{Router, RouterBuilderDiscoverExt, Slot, layout, page},
    view::{View, component, view},
};

struct AppEntry {
    href: &'static str,
    title: &'static str,
    description: &'static str,
}

const APP_ENTRIES: [AppEntry; 2] = [
    AppEntry {
        href: "/map",
        title: "Static map generator",
        description: "A customizable static map rendered by Rust",
    },
    AppEntry {
        href: "/fft",
        title: "Rust FFT benchmark",
        description: "RustFFT vs FFTW on different array lengths",
    },
];

pub fn router() -> Router {
    topcoat::router::module_router!()
        .discover()
        .assets(AssetBundle::load().unwrap())
        .build()
}

#[layout]
async fn root_layout(slot: Slot<'_>) -> Result {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            (slot.await?)
        </html>
    }
}

#[component]
pub(crate) async fn head(
    title: &str,
    description: &str,
    #[default(View::empty())] child: View,
) -> Result {
    view! {
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1">
            <meta name="description" content=(description)>
            <title>(title)</title>
            topcoat::font::link(font: fontsource_font!(INTER, host: Asset))
            <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
            topcoat::runtime::script()
            (child)
        </head>
    }
}

#[page]
async fn home() -> Result {
    view! {
        head(
            title: "Topcoat apps",
            description: "Small experiments built with Topcoat.",
        )
        <body>
            <main class="mx-auto flex min-h-screen max-w-2xl flex-col justify-center px-6 py-16">
                <h1 class="text-3xl font-semibold tracking-[-0.04em] text-stone-950">
                    "Small server-rendered experiments."
                </h1>
                <p class="mt-4">
                    "These are small apps built with Topcoat, a Rust framework for server-rendered web applications "
                    "with a unique "
                    <a target="_blank" class="underline" href="https://tokio.rs/blog/2026-07-22-announcing-topcoat">"interactivity model"</a>
                    "."
                </p>
                <p class="mt-4">
                    "Source code is available at "
                    <a target="_blank" class="underline" href="https://github.com/ekzhang/topcoat-apps">"github.com/ekzhang/topcoat-apps"</a>
                    "."
                </p>
                <div class="mt-10 space-y-3">
                    for app in &APP_ENTRIES {
                        <a
                            href=(app.href)
                            class="flex items-center justify-between rounded-xl border border-stone-200 bg-white px-5 py-4 text-sm font-semibold text-stone-900 shadow-sm transition hover:border-stone-300 hover:shadow"
                        >
                            <span>
                                <span class="block">(app.title)</span>
                                <span class="mt-1 block text-xs font-normal text-stone-500">
                                    (app.description)
                                </span>
                            </span>
                            <span class="text-stone-400" aria-hidden="true">"→"</span>
                        </a>
                    }
                </div>
            </main>
        </body>
    }
}
