use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use fuzzilua_gen::{all_generators, generate_program};
use fuzzilua_mutate::util::find_balanced_splice_ranges;
use rand::SeedableRng;
use rand::rngs::StdRng;

fn generate_test_programs(size: usize, count: usize) -> Vec<Vec<fuzzilua_ir::Instruction>> {
    let generators = all_generators();
    let mut rng = StdRng::seed_from_u64(0xBEEF_CAFE);
    (0..count)
        .map(|_| {
            let p = generate_program(&mut rng, size, 5, &generators);
            p.instructions
        })
        .collect()
}

fn bench_splice_ranges(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_balanced_splice_ranges");

    for &size in &[50, 150, 500] {
        let programs = generate_test_programs(size, 20);
        group.bench_with_input(BenchmarkId::from_parameter(size), &programs, |b, progs| {
            b.iter(|| {
                for p in progs {
                    std::hint::black_box(find_balanced_splice_ranges(p));
                }
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_splice_ranges);
criterion_main!(benches);
