# topcoat-apps

**<https://topcoat-apps.modal.ekzhang.com>**

Eric's demo apps made with the [Topcoat](https://github.com/tokio-rs/topcoat)
full-stack web framework in Rust.

Trying to showcase client-server reactivity to expose backend Rust logic in ways
that would otherwise be less ergonomic. We can build fully-featured apps and
interfaces without a separate frontend project and build step.

And maybe, we can even create high-performance variants of data visualizations
that would be too large for [Streamlit](https://streamlit.io/) or
[Pluto](https://plutojl.org/).

## Requirements

- Rust
- `cargo install topcoat-cli`
- FFTW system libraries: `brew install fftw`, `apt install libfftw3-dev`
