use criterion::{criterion_group, criterion_main};

fn manifest_benchmarks(_c: &mut criterion::Criterion) {
    // Benchmarks will be added as manifest pipeline is implemented.
}

criterion_group!(benches, manifest_benchmarks);
criterion_main!(benches);
