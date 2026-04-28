use criterion::{black_box, criterion_group, criterion_main, Criterion};
use reshell::compact;

fn generate_large_output(lines: usize) -> String {
    let mut s = String::new();
    for i in 0..lines {
        if i % 100 == 0 {
            s.push_str(&format!("fn function_{}() {{}}\n", i));
        } else if i % 50 == 0 {
            s.push_str(&format!("ERROR something went wrong at line {}\n", i));
        } else {
            s.push_str(&format!(
                "INFO line {} log message here with some padding\n",
                i
            ));
        }
    }
    s
}

fn bench_compaction(c: &mut Criterion) {
    let large = generate_large_output(10_000);
    c.bench_function("compact_10k_lines", |b| {
        b.iter(|| compact::compact(black_box(&large), None))
    });

    let huge = generate_large_output(100_000);
    c.bench_function("compact_100k_lines", |b| {
        b.iter(|| compact::compact(black_box(&huge), None))
    });
}

criterion_group!(benches, bench_compaction);
criterion_main!(benches);
