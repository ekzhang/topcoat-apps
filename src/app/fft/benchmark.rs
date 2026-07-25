//! Benchmark engine for the FFT route.

use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use fftw::{
    array::AlignedVec,
    plan::{C2CPlan, C2CPlan64},
    types::{Flag, Sign, c64},
};
use rustfft::{Fft, FftPlanner, num_complex::Complex64};

const MIN_INPUT_SIZE: usize = 64;
const MAX_INPUT_SIZE: usize = 4_096;
const INPUT_SIZE_STEP: usize = 16;

const MIN_SAMPLE_TIME: Duration = Duration::from_millis(3);
const MAX_BATCH_SIZE: usize = 4;
const SAMPLE_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Implementation {
    RustFft,
    Fftw,
}

impl Implementation {
    pub const ALL: [Self; 2] = [Self::RustFft, Self::Fftw];

    pub fn name(self) -> &'static str {
        match self {
            Self::RustFft => "RustFFT",
            Self::Fftw => "FFTW",
        }
    }

    pub fn color(self) -> &'static str {
        match self {
            Self::RustFft => "#2563eb",
            Self::Fftw => "#16a374",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Measurement {
    pub implementation: Implementation,
    pub nanos: f64,
    pub effective_gflops: f64,
}

#[derive(Clone, Debug)]
pub struct SizeResult {
    pub n: usize,
    pub factorization: String,
    pub family: &'static str,
    pub measurements: Vec<Measurement>,
}

#[derive(Clone, Debug)]
pub struct BenchmarkRun {
    pub rows: Vec<SizeResult>,
    pub elapsed: Duration,
}

impl SizeResult {
    pub fn measurement(&self, implementation: Implementation) -> Option<&Measurement> {
        self.measurements
            .iter()
            .find(|measurement| measurement.implementation == implementation)
    }

    pub fn fastest(&self) -> Option<&Measurement> {
        self.measurements.iter().min_by(|a, b| {
            a.nanos
                .partial_cmp(&b.nanos)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

pub fn run() -> BenchmarkRun {
    let started = Instant::now();
    let rows = input_sizes().into_iter().map(benchmark_size).collect();

    BenchmarkRun {
        rows,
        elapsed: started.elapsed(),
    }
}

fn input_sizes() -> Vec<usize> {
    let mut sizes = (MIN_INPUT_SIZE..=MAX_INPUT_SIZE)
        .step_by(INPUT_SIZE_STEP)
        .collect::<Vec<_>>();
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

fn benchmark_size(n: usize) -> SizeResult {
    // let sstart = Instant::now();

    let mut runners: Vec<Box<dyn KernelRunner>> = vec![Box::new(RustFftRunner::new(n))];
    runners.push(Box::new(FftwRunner::new(n)));

    // Calibrate every implementation before collecting samples so the first
    // series does not also pay for CPU wake-up and cold instruction caches.
    let batch_sizes = runners
        .iter_mut()
        .map(|runner| calibrate(runner.as_mut()))
        .collect::<Vec<_>>();
    let mut samples = vec![Vec::with_capacity(SAMPLE_COUNT); runners.len()];

    // Rotate the order each round. On heterogeneous CPUs this prevents one
    // implementation from always occupying the cold/slow-core time slot.
    for round in 0..SAMPLE_COUNT {
        for offset in 0..runners.len() {
            let index = (round + offset) % runners.len();
            samples[index].push(timed_sample(runners[index].as_mut(), batch_sizes[index]));
        }
    }

    let measurements = runners
        .iter()
        .zip(samples)
        .map(|(runner, mut samples)| {
            samples.sort_by(f64::total_cmp);
            let nanos_per_group = samples[SAMPLE_COUNT / 2];
            let nanos = nanos_per_group / runner.transforms_per_group() as f64;
            measurement(runner.implementation(), n, nanos)
        })
        .collect();

    // println!("finished bench {n} in {:?}", sstart.elapsed());

    SizeResult {
        n,
        factorization: factorization(n),
        family: size_family(n),
        measurements,
    }
}

fn measurement(implementation: Implementation, n: usize, nanos: f64) -> Measurement {
    let operations = 5.0 * n as f64 * (n as f64).log2();
    Measurement {
        implementation,
        nanos,
        effective_gflops: operations / nanos,
    }
}

trait KernelRunner {
    fn implementation(&self) -> Implementation;
    fn transforms_per_group(&self) -> usize;
    fn run_group(&mut self);
}

struct RustFftRunner {
    fft: Arc<dyn Fft<f64>>,
    input: Vec<Complex64>,
    output: Vec<Complex64>,
    scratch: Vec<Complex64>,
    transforms: usize,
}

impl RustFftRunner {
    fn new(n: usize) -> Self {
        let fft = FftPlanner::<f64>::new().plan_fft_forward(n);
        let transforms = transforms_per_sample(n);
        Self {
            input: vec![Complex64::new(0.0, 0.0); n * transforms],
            output: vec![Complex64::new(0.0, 0.0); n * transforms],
            scratch: vec![Complex64::new(0.0, 0.0); fft.get_immutable_scratch_len()],
            fft,
            transforms,
        }
    }
}

impl KernelRunner for RustFftRunner {
    fn implementation(&self) -> Implementation {
        Implementation::RustFft
    }

    fn transforms_per_group(&self) -> usize {
        self.transforms
    }

    fn run_group(&mut self) {
        self.fft
            .process_immutable_with_scratch(&self.input, &mut self.output, &mut self.scratch);
        black_box(&self.output);
    }
}

struct FftwRunner {
    plan: C2CPlan64,
    input: AlignedVec<c64>,
    output: AlignedVec<c64>,
    transforms: usize,
}

impl FftwRunner {
    fn new(n: usize) -> Self {
        Self {
            plan: C2CPlan64::aligned(&[n], Sign::Forward, Flag::ESTIMATE | Flag::PRESERVEINPUT)
                .expect("FFTW should create a plan for every positive input size"),
            input: AlignedVec::new(n),
            output: AlignedVec::new(n),
            transforms: transforms_per_sample(n),
        }
    }
}

impl KernelRunner for FftwRunner {
    fn implementation(&self) -> Implementation {
        Implementation::Fftw
    }

    fn transforms_per_group(&self) -> usize {
        self.transforms
    }

    fn run_group(&mut self) {
        for _ in 0..self.transforms {
            self.plan
                .c2c(&mut self.input, &mut self.output)
                .expect("FFTW plan and aligned buffers should stay compatible");
        }
        black_box(&self.output);
    }
}

fn transforms_per_sample(n: usize) -> usize {
    16384_usize.div_ceil(n)
}

fn calibrate(runner: &mut dyn KernelRunner) -> usize {
    runner.run_group();

    let mut batch_size = 1;
    loop {
        let started = Instant::now();
        for _ in 0..batch_size {
            runner.run_group();
        }
        if started.elapsed() >= MIN_SAMPLE_TIME || batch_size >= MAX_BATCH_SIZE {
            return batch_size;
        }
        batch_size *= 2;
    }
}

fn timed_sample(runner: &mut dyn KernelRunner, batch_size: usize) -> f64 {
    let started = Instant::now();
    for _ in 0..batch_size {
        runner.run_group();
    }
    started.elapsed().as_nanos() as f64 / batch_size as f64
}

fn prime_factors(mut n: usize) -> Vec<(usize, usize)> {
    let mut factors = Vec::new();
    let mut divisor = 2;

    while divisor * divisor <= n {
        let mut exponent = 0;
        while n.is_multiple_of(divisor) {
            n /= divisor;
            exponent += 1;
        }
        if exponent > 0 {
            factors.push((divisor, exponent));
        }
        divisor = if divisor == 2 { 3 } else { divisor + 2 };
    }
    if n > 1 {
        factors.push((n, 1));
    }

    factors
}

fn factorization(n: usize) -> String {
    prime_factors(n)
        .into_iter()
        .map(|(factor, exponent)| {
            if exponent == 1 {
                factor.to_string()
            } else {
                format!("{factor}^{exponent}")
            }
        })
        .collect::<Vec<_>>()
        .join(" × ")
}

fn size_family(n: usize) -> &'static str {
    let factors = prime_factors(n);
    if n.is_power_of_two() {
        "power of two"
    } else if factors.len() == 1 && factors[0].1 == 1 {
        "prime"
    } else if factors.iter().all(|(factor, _)| *factor <= 5) {
        "smooth"
    } else {
        "mixed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_input_shapes_and_batches() {
        let sizes = input_sizes();
        assert_eq!(sizes.len(), 253);
        assert_eq!(sizes.first(), Some(&64));
        assert_eq!(sizes.last(), Some(&4_096));
        assert!(sizes.contains(&80));
        assert!(!sizes.contains(&127));
        assert!(!sizes.contains(&2_039));
        assert_eq!(factorization(64), "2^6");
        assert_eq!(factorization(1000), "2^3 × 5^3");
        assert_eq!(size_family(127), "prime");
        assert_eq!(size_family(384), "smooth");
        assert_eq!(size_family(1717), "mixed");
        assert_eq!(transforms_per_sample(64), 256);
        assert_eq!(transforms_per_sample(1000), 17);
        assert_eq!(transforms_per_sample(8192), 2);
    }
}
