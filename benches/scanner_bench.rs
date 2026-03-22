use chrono::NaiveDate;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;
use timebomb::config::Config;
use timebomb::scanner::{build_regex, is_binary, scan_content, scan_str};

// ── Fixture builders ──────────────────────────────────────────────────────────

fn fixed_today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 6, 1).unwrap()
}

fn default_config() -> Config {
    Config::default()
}

/// Build a file with `total_lines` lines where annotations appear every
/// `annotation_every` lines. Pass `usize::MAX` to get zero annotations.
fn build_content(total_lines: usize, annotation_every: usize) -> String {
    let mut out = String::with_capacity(total_lines * 60);
    for i in 0..total_lines {
        if annotation_every < usize::MAX && i % annotation_every == 0 {
            // Half expired, half future — realistic mix
            let date = if i % 2 == 0 {
                "2020-01-01"
            } else {
                "2099-01-01"
            };
            let tag = ["TODO", "FIXME", "HACK", "TEMP", "REMOVEME"][i % 5];
            out.push_str(&format!(
                "    // {}[{}]: annotation number {}\n",
                tag, date, i
            ));
        } else {
            out.push_str(&format!("    let x_{} = {};\n", i, i));
        }
    }
    out
}

/// Build a file where every single line is an annotation.
fn build_dense_content(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 60);
    for i in 0..lines {
        let date = if i % 2 == 0 {
            "2020-01-01"
        } else {
            "2099-01-01"
        };
        out.push_str(&format!("    // TODO[{}]: dense annotation {}\n", date, i));
    }
    out
}

/// Build content that has NO `[` characters at all — exercises the pre-filter
/// fast path exclusively.
fn build_no_bracket_content(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 50);
    for i in 0..lines {
        out.push_str(&format!("    let variable_{} = {};\n", i, i));
    }
    out
}

/// Build content with `[` characters but no valid timebomb annotations —
/// exercises the regex on lines that pass the byte pre-filter but don't match.
fn build_bracket_no_annotation_content(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 60);
    for i in 0..lines {
        if i % 3 == 0 {
            out.push_str(&format!("    let arr[{}] = value;\n", i));
        } else if i % 3 == 1 {
            out.push_str(&format!("    // TODO: fix this [issue {}]\n", i));
        } else {
            out.push_str(&format!("    let x = foo[{}];\n", i));
        }
    }
    out
}

// ── scan_content benchmarks ───────────────────────────────────────────────────

fn bench_scan_content_no_annotations(c: &mut Criterion) {
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let today = fixed_today();
    let path = Path::new("bench/fake.rs");

    // No `[` at all — best case for the pre-filter
    let content_no_bracket = build_no_bracket_content(10_000);
    // Has `[` but no valid annotations — exercises regex on filtered lines
    let content_bracket = build_bracket_no_annotation_content(10_000);

    let mut group = c.benchmark_group("scan_content/no_annotations");
    group.throughput(Throughput::Elements(10_000));

    group.bench_function("no_bracket_10k_lines", |b| {
        b.iter(|| {
            scan_content(
                black_box(&content_no_bracket),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.bench_function("bracket_no_annotation_10k_lines", |b| {
        b.iter(|| {
            scan_content(
                black_box(&content_bracket),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.finish();
}

fn bench_scan_content_sparse(c: &mut Criterion) {
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let today = fixed_today();
    let path = Path::new("bench/fake.rs");

    // Annotation every 2000 lines → 5 annotations in 10k lines
    let content = build_content(10_000, 2_000);

    let mut group = c.benchmark_group("scan_content/sparse");
    group.throughput(Throughput::Elements(10_000));

    group.bench_function("5_annotations_10k_lines", |b| {
        b.iter(|| {
            scan_content(
                black_box(&content),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.finish();
}

fn bench_scan_content_density(c: &mut Criterion) {
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let today = fixed_today();
    let path = Path::new("bench/fake.rs");

    let mut group = c.benchmark_group("scan_content/by_density");

    for &(total, every, label) in &[
        (1_000, usize::MAX, "1k_lines_0_annotations"),
        (1_000, 100, "1k_lines_10_annotations"),
        (1_000, 10, "1k_lines_100_annotations"),
        (1_000, 1, "1k_lines_1000_annotations"),
        (10_000, usize::MAX, "10k_lines_0_annotations"),
        (10_000, 1_000, "10k_lines_10_annotations"),
        (10_000, 100, "10k_lines_100_annotations"),
        (10_000, 10, "10k_lines_1000_annotations"),
    ] {
        let content = build_content(total, every);
        group.throughput(Throughput::Elements(total as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            &content,
            |b, content| {
                b.iter(|| {
                    scan_content(
                        black_box(content),
                        black_box(path),
                        black_box(&regex),
                        black_box(&cfg),
                        black_box(today),
                    )
                    .unwrap()
                })
            },
        );
    }

    group.finish();
}

fn bench_scan_content_dense(c: &mut Criterion) {
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let today = fixed_today();
    let path = Path::new("bench/fake.rs");

    // Every line is an annotation
    let content_500 = build_dense_content(500);
    let content_2000 = build_dense_content(2_000);

    let mut group = c.benchmark_group("scan_content/dense");

    group.throughput(Throughput::Elements(500));
    group.bench_function("every_line_500_lines", |b| {
        b.iter(|| {
            scan_content(
                black_box(&content_500),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.throughput(Throughput::Elements(2_000));
    group.bench_function("every_line_2k_lines", |b| {
        b.iter(|| {
            scan_content(
                black_box(&content_2000),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.finish();
}

// ── scan_str benchmark (includes regex compile) ───────────────────────────────

fn bench_scan_str_includes_regex_compile(c: &mut Criterion) {
    let cfg = default_config();
    let today = fixed_today();
    let path = Path::new("bench/fake.rs");
    let content = build_content(1_000, 200); // 5 annotations in 1k lines

    let mut group = c.benchmark_group("scan_str");
    group.throughput(Throughput::Elements(1_000));

    // scan_str compiles the regex on every call — establishes the cost of
    // NOT caching the regex (the wrong pattern, as a baseline comparison).
    group.bench_function("1k_lines_5_annotations_with_regex_compile", |b| {
        b.iter(|| {
            scan_str(
                black_box(&content),
                black_box(path),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    // Same workload but regex pre-compiled — shows the amortised cost.
    let regex = build_regex(&cfg).unwrap();
    group.bench_function("1k_lines_5_annotations_precompiled_regex", |b| {
        b.iter(|| {
            scan_content(
                black_box(&content),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.finish();
}

// ── build_regex benchmark ─────────────────────────────────────────────────────

fn bench_build_regex(c: &mut Criterion) {
    let cfg_default = default_config();

    // Config with all 5 default tags
    let cfg_5_tags = Config {
        tags: vec![
            "TODO".into(),
            "FIXME".into(),
            "HACK".into(),
            "TEMP".into(),
            "REMOVEME".into(),
        ],
        ..Config::default()
    };

    // Config with a single tag — shorter alternation
    let cfg_1_tag = Config {
        tags: vec!["TODO".into()],
        ..Config::default()
    };

    // Config with many tags — longer alternation
    let cfg_10_tags = Config {
        tags: vec![
            "TODO".into(),
            "FIXME".into(),
            "HACK".into(),
            "TEMP".into(),
            "REMOVEME".into(),
            "NOTE".into(),
            "WARN".into(),
            "BUG".into(),
            "CLEANUP".into(),
            "DEPRECATED".into(),
        ],
        ..Config::default()
    };

    let mut group = c.benchmark_group("build_regex");

    group.bench_function("default_5_tags", |b| {
        b.iter(|| build_regex(black_box(&cfg_default)).unwrap())
    });

    group.bench_function("1_tag", |b| {
        b.iter(|| build_regex(black_box(&cfg_1_tag)).unwrap())
    });

    group.bench_function("5_tags", |b| {
        b.iter(|| build_regex(black_box(&cfg_5_tags)).unwrap())
    });

    group.bench_function("10_tags", |b| {
        b.iter(|| build_regex(black_box(&cfg_10_tags)).unwrap())
    });

    group.finish();
}

// ── is_binary benchmark ───────────────────────────────────────────────────────

fn bench_is_binary(c: &mut Criterion) {
    // Write two temp files: one plain text, one with a null byte near the end
    // of the 8 KB window. We time the full open+read+check cycle.
    let mut text_file = NamedTempFile::new().unwrap();
    text_file.write_all(&vec![b'a'; 8192]).unwrap();
    text_file.flush().unwrap();

    let mut binary_file = NamedTempFile::new().unwrap();
    let mut buf = vec![b'a'; 8192];
    buf[4096] = 0u8; // null byte in the middle
    binary_file.write_all(&buf).unwrap();
    binary_file.flush().unwrap();

    let text_path = text_file.path().to_path_buf();
    let binary_path = binary_file.path().to_path_buf();

    let mut group = c.benchmark_group("is_binary");

    group.bench_function("text_file_8kb", |b| {
        b.iter(|| is_binary(black_box(&text_path)).unwrap())
    });

    group.bench_function("binary_file_8kb", |b| {
        b.iter(|| is_binary(black_box(&binary_path)).unwrap())
    });

    group.finish();
}

// ── per-line micro-benchmarks ─────────────────────────────────────────────────

/// Benchmark the cost of a single regex match on a line known to contain an
/// annotation vs a line that doesn't. Isolates the per-line regex cost.
fn bench_per_line_regex(c: &mut Criterion) {
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let today = fixed_today();
    let path = Path::new("bench/fake.rs");

    // Single-line inputs
    let annotation_line = "    // TODO[2020-01-01]: remove this legacy code path\n";
    let plain_todo_line = "    // TODO: fix this later\n";
    let code_line = "    let result = compute_value(input, threshold);\n";
    let bracket_line = "    let arr = values[index];\n"; // has `[` but no annotation

    let mut group = c.benchmark_group("per_line");

    group.bench_function("annotation_line", |b| {
        b.iter(|| {
            scan_content(
                black_box(annotation_line),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.bench_function("plain_todo_no_date", |b| {
        b.iter(|| {
            scan_content(
                black_box(plain_todo_line),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.bench_function("code_line_no_bracket", |b| {
        b.iter(|| {
            scan_content(
                black_box(code_line),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.bench_function("bracket_line_no_annotation", |b| {
        b.iter(|| {
            scan_content(
                black_box(bracket_line),
                black_box(path),
                black_box(&regex),
                black_box(&cfg),
                black_box(today),
            )
            .unwrap()
        })
    });

    group.finish();
}

// ── annotation_regex_pattern benchmark ───────────────────────────────────────

fn bench_annotation_regex_pattern(c: &mut Criterion) {
    let cfg = default_config();

    c.bench_function("annotation_regex_pattern_build_string", |b| {
        b.iter(|| black_box(cfg.annotation_regex_pattern()))
    });
}

// ── registration ─────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_scan_content_no_annotations,
    bench_scan_content_sparse,
    bench_scan_content_density,
    bench_scan_content_dense,
    bench_scan_str_includes_regex_compile,
    bench_build_regex,
    bench_is_binary,
    bench_per_line_regex,
    bench_annotation_regex_pattern,
);
criterion_main!(benches);
