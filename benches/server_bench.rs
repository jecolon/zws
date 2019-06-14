#[macro_use]
extern crate criterion;

use criterion::black_box;
use criterion::Criterion;

use zws::server;

fn ctype_bench(c: &mut Criterion) {
    c.bench_function("ctype html", |b| {
        b.iter(|| server::get_ctype(black_box("a.html")))
    });
}

criterion_group!(benches, ctype_bench);
criterion_main!(benches);
