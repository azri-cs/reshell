use criterion::{black_box, criterion_group, criterion_main, Criterion};
use reshell::classify::taxonomy::RecoveryCode;
use reshell::env::Detector;
use reshell::memory::Store;
use reshell::recover::resolve::resolve_suggestion;

fn bench_recover_r22(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = Store::new_at_path(temp_dir.path().join("bench.db")).unwrap();
    let detector = Detector::default();

    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("recover_R22_command_not_found", |b| {
        b.iter(|| {
            let suggestion = rt.block_on(resolve_suggestion(
                &store,
                RecoveryCode::R22,
                black_box("gh pr view"),
                black_box("gh: command not found"),
                black_box(Some("gh: command not found")),
                &detector,
            ));
            let _ = black_box(suggestion);
        })
    });
}

fn bench_recover_r21_permission(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = Store::new_at_path(temp_dir.path().join("bench.db")).unwrap();
    let detector = Detector::default();

    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("recover_R21_permission_denied", |b| {
        b.iter(|| {
            let suggestion = rt.block_on(resolve_suggestion(
                &store,
                RecoveryCode::R21,
                black_box("rm /etc/hosts"),
                black_box("Permission denied"),
                black_box(Some("Permission denied")),
                &detector,
            ));
            let _ = black_box(suggestion);
        })
    });
}

fn bench_recover_r25_bashism(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = Store::new_at_path(temp_dir.path().join("bench.db")).unwrap();
    let detector = Detector::default();

    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("recover_R25_env_mismatch", |b| {
        b.iter(|| {
            let suggestion = rt.block_on(resolve_suggestion(
                &store,
                RecoveryCode::R25,
                black_box("[[ -f /tmp/foo && \"$x\" == \"bar\" ]]"),
                black_box("[[ not found"),
                black_box(Some("[[ not found")),
                &detector,
            ));
            let _ = black_box(suggestion);
        })
    });
}

criterion_group!(
    benches,
    bench_recover_r22,
    bench_recover_r21_permission,
    bench_recover_r25_bashism
);
criterion_main!(benches);
