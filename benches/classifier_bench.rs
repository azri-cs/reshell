use criterion::{black_box, criterion_group, criterion_main, Criterion};
use reshell::classify;

fn bench_classify_success(c: &mut Criterion) {
    c.bench_function("classify_R10", |b| {
        b.iter(|| {
            let result = classify::classify(0, black_box(""), black_box(""), false, black_box(""), None);
            black_box(result);
        })
    });
}

fn bench_classify_command_not_found(c: &mut Criterion) {
    c.bench_function("classify_R22", |b| {
        b.iter(|| {
            let result = classify::classify(
                127,
                black_box("gh: command not found"),
                black_box(""),
                false,
                black_box(""),
                None,
            );
            black_box(result);
        })
    });
}

fn bench_classify_permission_denied(c: &mut Criterion) {
    c.bench_function("classify_R21", |b| {
        b.iter(|| {
            let result = classify::classify(
                126,
                black_box("Permission denied"),
                black_box(""),
                false,
                black_box(""),
                None,
            );
            black_box(result);
        })
    });
}

fn bench_classify_npm_error(c: &mut Criterion) {
    c.bench_function("classify_R24_npm", |b| {
        b.iter(|| {
            let result = classify::classify(
                1,
                black_box("npm ERR! missing script: build\nnpm ERR! A complete log of this run"),
                black_box(""),
                false,
                black_box(""),
                None,
            );
            black_box(result);
        })
    });
}

fn bench_classify_normalize_bashisms(c: &mut Criterion) {
    c.bench_function("classify_R25_bashism", |b| {
        b.iter(|| {
            let result = classify::classify(
                1,
                black_box("bash: line 1: [[: command not found"),
                black_box(""),
                false,
                black_box("sh"),
                None,
            );
            black_box(result);
        })
    });
}

criterion_group!(
    benches,
    bench_classify_success,
    bench_classify_command_not_found,
    bench_classify_permission_denied,
    bench_classify_npm_error,
    bench_classify_normalize_bashisms,
);
criterion_main!(benches);
