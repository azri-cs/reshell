use criterion::{black_box, criterion_group, criterion_main, Criterion};
use reshell::exec::runner::Runner;
use reshell::exec::ExecRequest;
use reshell::memory::Store;
use std::collections::HashMap;

fn bench_exec_simple(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = Store::new_at_path(temp_dir.path().join("bench.db")).unwrap();
    let runner = Runner::with_store(store);

    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("exec_echo_hello", |b| {
        b.iter(|| {
            let request = ExecRequest {
                command: "echo hello".to_string(),
                cwd: None,
                timeout: 30,
                env: HashMap::new(),
                retry: false,
            };
            let result = rt.block_on(runner.run(black_box(&request)));
            let _ = black_box(result);
        })
    });
}

criterion_group!(benches, bench_exec_simple);
criterion_main!(benches);
