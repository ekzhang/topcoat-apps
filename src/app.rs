mod fft;

use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    font::fontsource::fontsource_font,
    router::{Router, RouterBuilderDiscoverExt, Slot, layout, page},
    view::{View, component, view},
};

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
                <h1 class="mt-3 text-3xl font-semibold tracking-[-0.04em] text-stone-950">
                    "Small server-rendered experiments."
                </h1>
                <a
                    href="/fft"
                    class="mt-10 flex items-center justify-between rounded-xl border border-stone-200 bg-white px-5 py-4 text-sm font-semibold text-stone-900 shadow-sm transition hover:border-stone-300 hover:shadow"
                >
                    <span>
                        <span class="block">"Rust FFT benchmark"</span>
                        <span class="mt-1 block text-xs font-normal text-stone-500">
                            "RustFFT vs FFTW on different array lengths"
                        </span>
                    </span>
                    <span class="text-stone-400" aria-hidden="true">"→"</span>
                </a>
            </main>
        </body>
    }
}
