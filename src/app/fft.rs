mod benchmark;

use std::{
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use benchmark::{BenchmarkRun, Implementation, SizeResult};
use tokio::sync::Semaphore;
use topcoat::{
    Result,
    router::page,
    runtime::shard,
    view::{component, view},
};

use super::head;

static BENCHMARK_SLOT: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
static INITIAL_RUN: LazyLock<Mutex<Option<CachedBenchmark>>> = LazyLock::new(|| Mutex::new(None));
const INITIAL_RUN_TTL: Duration = Duration::from_secs(60);

struct CachedBenchmark {
    created_at: Instant,
    run: BenchmarkRun,
}

#[page]
async fn fft_benchmark() -> Result {
    view! {
        head(
            title: "Rust FFT benchmark · Topcoat apps",
            description: "Interactive RustFFT and FFTW benchmarks across FFT input sizes.",
            <link
                rel="stylesheet"
                href=(topcoat::asset::asset!("assets/fft-chart.css"))
            >
            <script
                src=(topcoat::asset::asset!("assets/fft-chart.js"))
                defer="defer"
            ></script>
        )
        <body>
            signal run_id = 0.0;
            signal loading = false;
            signal show_runtime = false;

            <main
                class="mx-auto max-w-[1240px] px-5 py-8 sm:px-8 sm:py-12"
                @benchmarkcomplete=$(|_event| loading.set(false))
                :data-show-runtime=$(show_runtime.get())
            >

            <header class="mb-7 flex flex-col gap-5 sm:flex-row sm:items-end sm:justify-between">
                <div>
                    <h1 class="text-3xl font-semibold tracking-[-0.045em] text-stone-950 sm:text-4xl">
                        "Rust FFT benchmark"
                    </h1>
                    <p class="mt-2 max-w-2xl text-sm leading-6 text-stone-500">
                        "Single-threaded RustFFT and FFTW on complex float64 transforms. Runs on the server."
                    </p>
                </div>
                <div class="flex flex-wrap items-center gap-2">
                    <div
                        role="group"
                        aria-label="Chart metric"
                        class="inline-flex h-10 items-center rounded-lg bg-stone-200/70 p-1"
                    >
                        <button
                            type="button"
                            @click=$(|_event| show_runtime.set(false))
                            :disabled=$(!show_runtime.get())
                            :aria-pressed=$(!show_runtime.get())
                            class="h-8 rounded-md px-3 text-sm font-semibold text-stone-500 transition hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-stone-500 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                        >
                            "Speed"
                        </button>
                        <button
                            type="button"
                            @click=$(|_event| show_runtime.set(true))
                            :disabled=$(show_runtime.get())
                            :aria-pressed=$(show_runtime.get())
                            class="h-8 rounded-md px-3 text-sm font-semibold text-stone-500 transition hover:text-stone-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-stone-500 disabled:cursor-default disabled:bg-white disabled:text-stone-950 disabled:shadow-sm"
                        >
                            "Runtime"
                        </button>
                    </div>
                    <button
                        type="button"
                        @click=$(|_event| {
                            loading.set(true);
                            run_id.set(run_id.get() + 1.0);
                        })
                        :disabled=$(loading.get())
                        :aria-busy=$(loading.get())
                        class="inline-flex h-10 min-w-28 shrink-0 items-center justify-center gap-2 rounded-lg bg-stone-950 px-4 text-sm font-semibold text-white transition hover:bg-stone-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-stone-500 focus-visible:ring-offset-2 disabled:cursor-wait disabled:bg-stone-500"
                    >
                        <span
                            :hidden=$(!loading.get())
                            aria-hidden="true"
                            class="size-4 animate-spin rounded-full border-2 border-white/35 border-t-white"
                        ></span>
                        <span :hidden=$(loading.get())>"Run again"</span>
                        <span :hidden=$(!loading.get())>"Running…"</span>
                    </button>
                </div>
            </header>

                <div
                    :class=$(loading.get().then_some("opacity-60"))
                    :aria-busy=$(loading.get())
                >
                    benchmark_results(run_id: $(run_id.get()))
                </div>
            </main>
        </body>
    }
}

#[shard]
async fn benchmark_results(run_id: f64) -> Result {
    let run_id = if run_id.is_finite() && (0.0..=10_000.0).contains(&run_id) {
        run_id.round() as u64
    } else {
        0
    };

    let permit = BENCHMARK_SLOT.clone().acquire_owned().await?;
    let run = tokio::task::spawn_blocking(move || {
        let _permit = permit;

        if run_id == 0 {
            let cached = INITIAL_RUN
                .lock()
                .expect("initial benchmark cache should not be poisoned")
                .as_ref()
                .filter(|cached| cached.created_at.elapsed() <= INITIAL_RUN_TTL)
                .map(|cached| cached.run.clone());
            if let Some(run) = cached {
                return run;
            }
        }

        let run = benchmark::run();
        if run_id == 0 {
            *INITIAL_RUN
                .lock()
                .expect("initial benchmark cache should not be poisoned") = Some(CachedBenchmark {
                created_at: Instant::now(),
                run: run.clone(),
            });
        }
        run
    })
    .await?;

    view! {
        benchmark_dashboard(run_id: run_id, run: &run)
    }
}

#[component]
async fn benchmark_dashboard(run_id: u64, run: &BenchmarkRun) -> Result {
    let run_label = if run_id == 0 {
        "initial run".to_string()
    } else {
        format!("run #{run_id}")
    };
    let summary = format!(
        "{} · {} sizes",
        format_duration(run.elapsed),
        run.rows.len()
    );

    view! {
        <img
            src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw=="
            alt=""
            aria-hidden="true"
            class="hidden"
            onload="this.dispatchEvent(new Event('benchmarkcomplete', { bubbles: true }))"
        >
        <section aria-live="polite" class="space-y-5">
            <div class="overflow-hidden rounded-xl border border-stone-200 bg-white">
                <div class="flex flex-col gap-4 border-b border-stone-200 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
                    <div>
                        <h2 class="text-sm font-semibold text-stone-950">
                            <span class="metric-throughput-label">"Speed (GFLOP/s) by array size"</span>
                            <span class="metric-runtime-label">"Latency by array size"</span>
                        </h2>
                        <p class="mt-1 text-xs text-stone-600">
                            (run_label)
                            " · "
                            (summary)
                        </p>
                    </div>
                    <div class="flex flex-wrap gap-4">
                        for implementation in Implementation::ALL {
                            <span class="flex items-center gap-2 text-xs font-medium text-stone-600">
                                <span
                                    class="size-2 rounded-full"
                                    style=(format!("background: {}", implementation.color()))
                                ></span>
                                (implementation.name())
                            </span>
                        }
                    </div>
                </div>
                <div class="metric-throughput-chart p-2 sm:p-5">
                    benchmark_chart(run: run, metric: ChartMetric::Throughput)
                </div>
                <div class="metric-runtime-chart p-2 sm:p-5">
                    benchmark_chart(run: run, metric: ChartMetric::Runtime)
                </div>
            </div>

            benchmark_table(run: run)
        </section>
    }
}

#[derive(Clone, Copy, Debug)]
enum ChartMetric {
    Throughput,
    Runtime,
}

impl ChartMetric {
    fn value(self, nanos: f64, gflops: f64) -> f64 {
        match self {
            Self::Throughput => gflops,
            Self::Runtime => nanos / 1_000.0,
        }
    }

    fn axis_label(self) -> &'static str {
        match self {
            Self::Throughput => "Speed (GFLOP/s)",
            Self::Runtime => "Latency (µs)",
        }
    }

    fn tooltip_value(self, nanos: f64, gflops: f64) -> String {
        match self {
            Self::Throughput => format!("{gflops:.2} GFLOP/s"),
            Self::Runtime => format_latency(nanos),
        }
    }
}

#[derive(Debug)]
struct ChartPoint {
    x: f64,
    y: f64,
    n: usize,
    factorization: String,
    nanos: f64,
    gflops: f64,
}

#[derive(Debug)]
struct ChartSeries {
    implementation: Implementation,
    path: String,
    points: Vec<ChartPoint>,
}

#[component]
async fn benchmark_chart(run: &BenchmarkRun, metric: ChartMetric) -> Result {
    const WIDTH: f64 = 1120.0;
    const HEIGHT: f64 = 420.0;
    const LEFT: f64 = 72.0;
    const RIGHT: f64 = 18.0;
    const TOP: f64 = 20.0;
    const BOTTOM: f64 = 62.0;

    let plot_width = WIDTH - LEFT - RIGHT;
    let plot_height = HEIGHT - TOP - BOTTOM;
    let x_max = run.rows.last().map(|row| row.n as f64).unwrap_or(1.0);
    let observed_max = run
        .rows
        .iter()
        .flat_map(|row| &row.measurements)
        .map(|measurement| metric.value(measurement.nanos, measurement.effective_gflops))
        .fold(0.0_f64, f64::max);
    let (y_max, y_tick_step) = chart_y_axis(observed_max);
    let x_for = |n: usize| LEFT + n as f64 / x_max * plot_width;
    let y_for = |value: f64| TOP + (1.0 - value / y_max) * plot_height;

    let x_ticks = (0..=4)
        .map(|index| {
            let n = (x_max * index as f64 / 4.0).round() as usize;
            (x_for(n), format_input_size(n))
        })
        .collect::<Vec<_>>();
    let y_tick_count = (y_max / y_tick_step).round() as usize;
    let y_ticks = (0..=y_tick_count)
        .map(|index| {
            let value = y_tick_step * index as f64;
            (y_for(value), value)
        })
        .collect::<Vec<_>>();
    let series = Implementation::ALL
        .into_iter()
        .map(|implementation| {
            let points = run
                .rows
                .iter()
                .filter_map(|row| {
                    row.measurement(implementation)
                        .map(|measurement| ChartPoint {
                            x: x_for(row.n),
                            y: y_for(metric.value(measurement.nanos, measurement.effective_gflops)),
                            n: row.n,
                            factorization: row.factorization.clone(),
                            nanos: measurement.nanos,
                            gflops: measurement.effective_gflops,
                        })
                })
                .collect::<Vec<_>>();
            let path = points
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    format!(
                        "{}{:.2},{:.2}",
                        if index == 0 { "M" } else { " L" },
                        point.x,
                        point.y
                    )
                })
                .collect::<String>();
            ChartSeries {
                implementation,
                path,
                points,
            }
        })
        .collect::<Vec<_>>();

    view! {
        <div class="w-full overflow-x-auto">
            <svg
                viewBox=(format!("0 0 {WIDTH:.0} {HEIGHT:.0}"))
                aria-hidden="true"
                data-fft-chart=""
                class="block h-auto w-full min-w-[720px]"
            >
                for (y, value) in y_ticks {
                    <line
                        x1=(LEFT)
                        y1=(y)
                        x2=(LEFT + plot_width)
                        y2=(y)
                        stroke="#e7e5e4"
                        stroke-width="1"
                    ></line>
                    <text
                        x=(LEFT - 12.0)
                        y=(y + 4.0)
                        text-anchor="end"
                        fill="#a8a29e"
                        font-size="11"
                    >
                        (format!("{value:.0}"))
                    </text>
                }

                for (x, label) in x_ticks {
                    <line
                        x1=(x)
                        y1=(TOP)
                        x2=(x)
                        y2=(TOP + plot_height)
                        stroke="#e7e5e4"
                        stroke-width="1"
                    ></line>
                    <text
                        x=(x)
                        y=(TOP + plot_height + 24.0)
                        text-anchor="middle"
                        fill="#a8a29e"
                        font-size="11"
                    >
                        (label)
                    </text>
                }

                for chart_series in &series {
                    <path
                        d=(chart_series.path.as_str())
                        fill="none"
                        stroke=(chart_series.implementation.color())
                        stroke-opacity="0.3"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                    ></path>
                    for point in &chart_series.points {
                        <circle
                            cx=(point.x)
                            cy=(point.y)
                            r="4"
                            data-chart-point=""
                            data-n=(point.n)
                            data-factors=(point.factorization.as_str())
                            data-series=(chart_series.implementation.name())
                            data-value=(metric.tooltip_value(point.nanos, point.gflops))
                            fill=(chart_series.implementation.color())
                            stroke="white"
                            stroke-width="1.5"
                        ></circle>
                    }
                }

                <line
                    data-chart-guide=""
                    x1=(LEFT)
                    y1=(TOP)
                    x2=(LEFT)
                    y2=(TOP + plot_height)
                    stroke="#78716c"
                    stroke-width="1"
                    stroke-dasharray="3 4"
                    pointer-events="none"
                ></line>

                <text
                    x="18"
                    y=(TOP + plot_height / 2.0)
                    transform=(format!(
                        "rotate(-90 18 {:.2})",
                        TOP + plot_height / 2.0
                    ))
                    text-anchor="middle"
                    fill="#78716c"
                    font-size="13"
                    font-weight="600"
                >
                    (metric.axis_label())
                </text>
                <text
                    x=(LEFT + plot_width / 2.0)
                    y=(HEIGHT - 8.0)
                    text-anchor="middle"
                    fill="#78716c"
                    font-size="13"
                    font-weight="600"
                >
                    "input length N"
                </text>
            </svg>
        </div>
    }
}

#[component]
async fn benchmark_table(run: &BenchmarkRun) -> Result {
    view! {
        <section class="overflow-hidden rounded-xl border border-stone-200 bg-white">
            <div class="border-b border-stone-200 px-5 py-3">
                <h2 class="text-sm font-semibold text-stone-950">"Input breakdown"</h2>
            </div>
            <div data-benchmark-table-scroll="" class="max-h-[480px] overflow-auto">
                <table class="w-full min-w-[680px] border-collapse text-left">
                    <thead class="sticky top-0 z-10 bg-stone-50 text-[10px] uppercase tracking-[0.12em] text-stone-400">
                        <tr>
                            <th class="px-5 py-3 font-semibold">"N"</th>
                            <th class="px-3 py-3 font-semibold">"factors"</th>
                            <th class="px-3 py-3 font-semibold">"shape"</th>
                            for implementation in Implementation::ALL {
                                <th class="px-3 py-3 text-right font-semibold">
                                    (implementation.name())
                                </th>
                            }
                        </tr>
                    </thead>
                    <tbody class="divide-y divide-stone-100 text-xs">
                        for row in &run.rows {
                            benchmark_row(row: row)
                        }
                    </tbody>
                </table>
            </div>
        </section>
    }
}

#[component]
async fn benchmark_row(row: &SizeResult) -> Result {
    let fastest = row.fastest().map(|measurement| measurement.implementation);

    view! {
        <tr data-benchmark-row="" data-n=(row.n) class="benchmark-row hover:bg-stone-50">
            <td class="px-5 py-3 font-semibold tabular-nums text-stone-900">(row.n)</td>
            <td class="px-3 py-3 font-mono text-stone-500">(row.factorization.as_str())</td>
            <td class="px-3 py-3 text-stone-400">(row.family)</td>
            for implementation in Implementation::ALL {
                <td class="px-3 py-3 text-right tabular-nums">
                    if let Some(measurement) = row.measurement(implementation) {
                        <span class=(if fastest == Some(implementation) {
                            "font-semibold text-emerald-700"
                        } else {
                            "text-stone-500"
                        })>
                            (format_latency(measurement.nanos))
                        </span>
                    } else {
                        <span class="text-stone-300" title="Not measured">"—"</span>
                    }
                </td>
            }
        </tr>
    }
}

fn chart_y_axis(observed_max: f64) -> (f64, f64) {
    if !observed_max.is_finite() || observed_max <= 0.0 {
        return (1.0, 0.2);
    }

    const TARGET_INTERVALS: f64 = 4.0;
    const HEADROOM: f64 = 1.05;

    let rough_step = observed_max / TARGET_INTERVALS;
    let magnitude = 10_f64.powf(rough_step.log10().floor());
    let normalized = rough_step / magnitude;
    let nice_step = if normalized < 2.0_f64.sqrt() {
        1.0
    } else if normalized < 10.0_f64.sqrt() {
        2.0
    } else if normalized < 50.0_f64.sqrt() {
        5.0
    } else {
        10.0
    } * magnitude;

    let y_max = (observed_max * HEADROOM / nice_step).ceil() * nice_step;
    (y_max.max(nice_step), nice_step)
}

fn format_input_size(n: usize) -> String {
    if n >= 1024 {
        format!("{}k", n / 1024)
    } else {
        n.to_string()
    }
}

fn format_latency(nanos: f64) -> String {
    if nanos < 1_000.0 {
        format!("{nanos:.0} ns")
    } else if nanos < 1_000_000.0 {
        format!("{:.1} µs", nanos / 1_000.0)
    } else {
        format!("{:.2} ms", nanos / 1_000_000.0)
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs_f64() < 1.0 {
        format!("{:.0} ms", duration.as_secs_f64() * 1_000.0)
    } else {
        format!("{:.2} s", duration.as_secs_f64())
    }
}
