use criterion::{criterion_group, criterion_main};

fn ecs_benchmarks(_c: &mut criterion::Criterion) {
    // Benchmarks will be added as ECS components are implemented.
}

criterion_group!(benches, ecs_benchmarks);
criterion_main!(benches);
