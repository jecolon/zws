#[macro_use]
extern crate criterion;

use criterion::Criterion;

fn dummy_bench(c: &mut Criterion) {
    c.bench_function("dummy", |b| b.iter(|| 2 + 2));
}

criterion_group!(benches, dummy_bench);
criterion_main!(benches);
