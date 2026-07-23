use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::Cx,
    font::fontsource::fontsource_font,
    router::{Router, RouterBuilderDiscoverExt, Slot, layout, page, query_params},
    view::{component, view},
};

#[tokio::main]
async fn main() {
    let router = Router::builder()
        .discover()
        .assets(AssetBundle::load().unwrap())
        .build();

    topcoat::start(router).await.unwrap();
}

#[layout("/")]
async fn layout(slot: Slot<'_>) -> Result {
    view! {
        <!DOCTYPE html>
        <html>
            <head>
                topcoat::font::link(font: fontsource_font!(INTER, host: Asset))
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
            </head>
            <body>(slot.await?)</body>
        </html>
    }
}

#[query_params(error = bad_request)]
struct PostQuery {
    name: Option<String>,
}

#[page("/")]
async fn home(cx: &Cx) -> Result {
    let query = query_params::<PostQuery>(cx)?;
    view! {
        <main class="p-4">hello(name: query.name.as_deref().unwrap_or("World"))</main>
    }
}

#[component]
async fn hello(name: &str) -> Result {
    view! {
        <h1 class="text-xl">
            "Hello, "
            (name)
            "!"
        </h1>
    }
}
