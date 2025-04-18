#!/usr/bin/env just --justfile

results := 'results'
random_actions_results := results / "random-actions-1threads.csv"
heap_efficiency_results := results / "heap-efficiency.csv"
memory_efficiency_results := results / "memory-efficiency.csv"
microbench_results := results / "microbench.csv"
wasm_size_results := results / "wasm-size.csv"
wasm_perf_results := results / "wasm-perf.csv"

plots := 'plots'
random_actions_plot := plots / "random-actions-1threads.png"
heap_efficiency_plot := plots / "heap-efficiency.png"
memory_efficiency_plot := plots / "memory-efficiency.png"
microbench_plot := plots / "microbench.png"
wasm_size_plot := plots / "wasm-size.png"
wasm_perf_plot := plots / "wasm-perf.png"

default:
    @just --list

check: && check-wasm-size check-wasm-perf
    #!/usr/bin/env bash
    set -euxo pipefail 2>/dev/null
    # STABLE CONFIGURATIONS
    rustup run stable cargo check -p talc
    rustup run stable cargo check -p talc --features=disable-grow-in-place
    rustup run stable cargo check -p talc --features=disable-realloc-in-place
    rustup run stable cargo check -p talc --features=disable-grow-in-place,disable-realloc-in-place
    rustup run stable cargo check -p talc --features=counters
    rustup run stable cargo check -p talc --features=counters,cache-aligned-allocations
    rustup run stable cargo check -p talc --target wasm32-unknown-unknown
    rustup run stable cargo check -p talc --features=disable-grow-in-place --target wasm32-unknown-unknown
    rustup run stable cargo check -p talc --features=disable-realloc-in-place --target wasm32-unknown-unknown
    # check that the examples aren't broken
    rustup run stable cargo check -p talc --example allocator_api
    rustup run stable cargo check -p talc --example global_allocator
    # TODO ADD THE OTHERS
    # check whether MSRV has been broken
    rustup run 1.63.0 cargo check -p talc --features=counters
    # check the benchmarks
    rustup run nightly cargo check -p benches --bin microbench
    rustup run nightly cargo check -p benches --bin random_actions
    rustup run nightly cargo check -p benches --bin heap_efficiency
    # test everything
    rustup run stable cargo test -p talc --all-targets --features=counters
    rustup run stable cargo test -p talc --doc --features=counters
    rustup run stable cargo test -p talc --all-targets --features=disable-realloc-in-place,cache-aligned-allocations
    rustup run stable cargo test -p talc --doc --features=disable-realloc-in-place,cache-aligned-allocations
    # miri
    rustup run nightly cargo miri test -p talc --all-targets --features=counters
    rustup run nightly cargo miri test -p talc --doc --features=counters
    rustup run nightly cargo miri test -p talc --all-targets --target i686-unknown-linux-gnu --features=disable-realloc-in-place,cache-aligned-allocations
    rustup run nightly cargo miri test -p talc --doc --target i686-unknown-linux-gnu --features=disable-realloc-in-place,cache-aligned-allocations
    # nightly
    rustup run nightly cargo test -p talc --features=nightly,counters

test:
    #!/usr/bin/env sh
    set -euxo pipefail 2>/dev/null
    rustup run stable cargo check -p talc

fuzz:
    cargo fuzz run fuzz_talc

random-actions: && (plot-random-actions "random-actions")
    cargo run -p benches --bin random_actions --release -- --name "random-actions"

random-actions-no-realloc: && (plot-random-actions-no-realloc "random-actions-no-realloc")
    cargo run -p benches --bin random_actions --release -- --name "random-actions-no-realloc" --no-realloc

random-actions-multi: && (plot-random-actions "random-actions-multi")
    cargo run -p benches --bin random_actions --release -- --name "random-actions-multi" --thread-count 4

random-actions-sys: && (plot-random-actions "random-actions-sys")
    cargo run -p benches --bin random_actions --release -- --name "random-actions-sys" --system

random-actions-sys-no-realloc: && (plot-random-actions-no-realloc "random-actions-sys-no-realloc")
    cargo run -p benches --bin random_actions --release -- --name "random-actions-sys-no-realloc" --no-realloc --system

random-actions-sys-multi: && (plot-random-actions "random-actions-sys-multi")
    cargo run -p benches --bin random_actions --release -- --name "random-actions-sys-multi" --thread-count 4 --system

plot-random-actions name:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    with open("{{results}}/{{name}}.csv", 'r') as f:
        rows = f.readlines()
    max_sizes = [int(i) for i in rows[0].split(',')[1:]]
    allocators = []
    for row in rows[1:]:
        lst = row.split(',')
        allocators.append(([lst[0]], [float(i) for i in lst[1:]]))
    allocators.sort(key=lambda a: -a[1][0])
    maxima = [0 for i in range(len(allocators[0][1]))]
    for k, v in allocators:
        for i, y in enumerate(v):
            maxima[i] = max(maxima[i], y)
    for k, v in allocators:
        for i in range(len(v)):
            v[i] = v[i] / maxima[i]
    yvalues = []
    for k, v in allocators:
        plt.plot(max_sizes, v, label=k)
        yvalues.append(v)
    plt.xscale('log')
    # plt.yscale('log')
    plt.legend()
    plt.title("Random Actions Benchmark")
    plt.xticks(ticks=max_sizes, labels=[str(x) + " / " + str(x*3) for x in max_sizes], rotation=15)
    plt.xlabel('Max Allocation Size (bytes) / Max Reallocation Size (bytes)')
    plt.ylabel('Relative Score')
    plt.tight_layout()
    plt.savefig("{{plots}}/{{name}}.png")
    plt.show()

plot-random-actions-no-realloc name:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    with open("{{results}}/{{name}}.csv", 'r') as f:
        rows = f.readlines()
    max_sizes = [int(i) for i in rows[0].split(',')[1:]]
    allocators = []
    for row in rows[1:]:
        lst = row.split(',')
        allocators.append(([lst[0]], [float(i) for i in lst[1:]]))
    allocators.sort(key=lambda a: -a[1][0])
    maxima = [0 for i in range(len(allocators[0][1]))]
    for k, v in allocators:
        for i, y in enumerate(v):
            maxima[i] = max(maxima[i], y)
    for k, v in allocators:
        for i in range(len(v)):
            v[i] = v[i] / maxima[i]
    yvalues = []
    for k, v in allocators:
        plt.plot(max_sizes, v, label=k)
        yvalues.append(v)
    plt.xscale('log')
    plt.legend()
    plt.title("Random Actions Benchmark (No Reallocation)")
    plt.xticks(ticks=max_sizes, labels=[str(x) for x in max_sizes], rotation=15)
    plt.xlabel('Max Allocation Size (bytes)')
    plt.ylabel('Relative Score')
    plt.tight_layout()
    plt.savefig("{{plots}}/{{name}}.png")
    plt.show()

microbench: && plot-microbench
    cargo run -p benches --bin microbench --release

plot-microbench:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    from matplotlib.lines import Line2D
    with open("{{microbench_results}}", 'r') as f:
        rows = f.readlines()
    allocators = []
    for row in rows:
        lst = row.strip().split(',')
        if lst[1] != "":
            allocators.append((lst[0], [float(i) for i in lst[1:]]))
    # sort by median
    allocators.sort(key=lambda a: -a[1][2])
    rightlim = max([x[1][3] for x in allocators])*1.2
    plt.boxplot([x[1] for x in allocators], sym="", vert=False, showmeans=False, meanline=False, whis=(0, 100))
    i = 1
    for al in allocators:
        plt.annotate(str(int(al[1][4])), (rightlim - 50, i), )
        i += 1
    plt.title("Allocation and Deallocation Micro-benchmark")
    plt.xlim(left=0, right=rightlim)
    plt.yticks(range(1, len(allocators) + 1), [x[0] for x in allocators])
    plt.xlabel("Ticks")
    plt.tight_layout()
    plt.savefig("{{microbench_plot}}")
    plt.show()

heap-efficiency: && plot-heap-efficiency
    cargo run -p benches --bin heap_efficiency --release

plot-heap-efficiency:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    with open("{{heap_efficiency_results}}", 'r') as f:
        rows = f.readlines()
    names = list(filter(None, rows[0].split(',')))
    efficiencies = list(map(float, filter(None, rows[1].split(','))))
    names, efficiencies = (list(x) for x in zip(*sorted(zip(names, efficiencies), key=lambda pair: -pair[1])))
    plt.bar(names, efficiencies)
    plt.title('Heap Efficiency')
    plt.ylabel('Average Percentage of Heap Used before OOM %')
    plt.tight_layout()
    plt.savefig("{{heap_efficiency_plot}}")
    plt.show()

memory-efficiency: && plot-memory-efficiency
    cargo run -p benches --bin memory_efficiency --release

plot-memory-efficiency:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    import numpy as np
    with open("{{memory_efficiency_results}}", 'r') as f:
        rows = f.readlines()
    names = rows[0].split(',')
    tris = list(filter(None, rows[1].split(',')))
    allocated = list(map(float, map(lambda x: x.strip().split(' ')[0], tris)))
    used_phys = list(map(float, map(lambda x: x.strip().split(' ')[1], tris)))
    used_virt = list(map(float, map(lambda x: x.strip().split(' ')[2], tris)))
    names, allocated, used_phys, used_virt = (list(x) for x in zip(*sorted(zip(names, allocated, used_phys, used_virt), key=lambda quad: -quad[3])))
    y = np.arange(len(names))
    w = 0.3
    f = plt.figure()
    f.set_figwidth(8)
    plt.barh(y+0.3, allocated, w, color='orange')
    plt.barh(y+0, used_phys, w, color='black')
    plt.barh(y-0.3, used_virt, w, color='green')
    plt.yticks(y, names);
    plt.title('Memory Efficiency Benchmark (Lower is Better)')
    plt.xlabel('Memory Usage At Cutoff')
    plt.legend(["Allocated", "Physical Memory Usage", "Virtual Memory Usage"])
    plt.tight_layout()
    plt.savefig("{{memory_efficiency_plot}}")
    plt.show()

check-wasm-size:
    #!/usr/bin/env bash
    set -euxo pipefail 2>/dev/null
    features="no_alloc talc talc_arena rlsf rlsf_small dlmalloc lol_alloc"
    for feature in ${features}; do
        cargo +nightly check -p wasm-size --quiet --release --target wasm32-unknown-unknown --features ${feature}
    done

wasm-size: && plot-wasm-size
    #!/usr/bin/env bash
    set -euxo pipefail 2>/dev/null
    printf "No Allocator,Talc (Dynamic),Talc (Dynamic disable-grow-in-place),Talc (Dynamic disable-realloc-in-place),Talc (Arena),Talc (Arena disable-grow-in-place),Talc (Arena disable-realloc-in-place),RLSF (Normal),RLSF (Small),DLMalloc,lol_alloc\n" > {{wasm_size_results}}
    features="no_alloc talc talc_no_grow talc_no_realloc talc_arena talc_arena_no_grow talc_arena_no_realloc rlsf rlsf_small dlmalloc lol_alloc"
    for feature in ${features}; do
        RUSTFLAGS="-C lto -C embed-bitcode=yes -C linker-plugin-lto" \
            cargo +nightly build -p wasm-size --quiet --release --target wasm32-unknown-unknown --features ${feature}
        wasm-opt -Oz -o target/wasm32-unknown-unknown/release/wasm_size_opt.wasm target/wasm32-unknown-unknown/release/wasm_size.wasm
        printf "$(wc -c  < ./target/wasm32-unknown-unknown/release/wasm_size_opt.wasm)," >> {{wasm_size_results}}
    done
    truncate -s-1 {{wasm_size_results}} # remove the trailing comma

plot-wasm-size:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    with open("{{wasm_size_results}}", 'r') as f:
        rows = f.readlines()
    names = rows[0].split(',')
    sizes = list(map(float, filter(None, rows[1].split(','))))
    names, sizes = (list(x) for x in zip(*sorted(zip(names, sizes), key=lambda pair: -pair[1])))
    plt.barh(names, sizes)
    plt.title('WebAssembly Module Size (Lower is Better)')
    plt.xlabel('Bytes')
    plt.xlim(xmin=0)
    plt.tight_layout()
    plt.savefig("{{wasm_size_plot}}")
    plt.show()


check-wasm-perf:
    #!/usr/bin/env bash
    set -euxo pipefail 2>/dev/null
    features="talc talc_arena rlsf rlsf_small dlmalloc lol_alloc"
    for feature in ${features}; do
        wasm-pack --quiet --log-level warn build wasm-perf --release --target deno --features ${feature}
    done

wasm-perf: && plot-wasm-perf
    #!/usr/bin/env bash
    set -euxo pipefail 2>/dev/null
    # printf "Talc (Dynamic),Talc (Arena),RLSF (Normal),RLSF (Small),DLMalloc,lol_alloc\n" > {{wasm_perf_results}}
    # features="talc talc_arena rlsf rlsf_small dlmalloc lol_alloc"
    printf "Talc (Dynamic),Talc (Dynamic disable-grow-in-place),Talc (Dynamic disable-realloc-in-place),Talc (Arena),Talc (Arena disable-grow-in-place),Talc (Arena disable-realloc-in-place),RLSF (Normal),RLSF (Small),DLMalloc,lol_alloc\n" > {{wasm_perf_results}}
    features="talc talc_no_grow talc_no_realloc talc_arena talc_arena_no_grow talc_arena_no_realloc rlsf rlsf_small dlmalloc lol_alloc"
    for feature in ${features}; do
        wasm-pack --quiet --log-level warn build wasm-perf --release --target deno --features ${feature}
        printf "$(deno run --allow-read wasm-perf/bench.js)," >> {{wasm_perf_results}}
    done
    truncate -s-1 {{wasm_perf_results}} # remove the trailing comma

plot-wasm-perf:
    #!/usr/bin/env python
    import matplotlib.pyplot as plt
    import numpy as np
    with open("{{wasm_perf_results}}", 'r') as f:
        rows = f.readlines()
    names = rows[0].split(',')
    pairs = list(filter(None, rows[1].split(',')))
    alloc_dealloc = list(map(float, map(lambda x: x.strip().split(' ')[0], pairs)))
    ad_realloc = list(map(float, map(lambda x: x.strip().split(' ')[1], pairs)))
    names, alloc_dealloc, ad_realloc = (list(x) for x in zip(*sorted(zip(names, alloc_dealloc, ad_realloc), key=lambda tri: tri[1] + tri[2])))
    y = np.arange(len(names))
    w = 0.4
    f = plt.figure()
    f.set_figwidth(8)
    plt.barh(y+0.2, alloc_dealloc, w, color='orange')
    plt.barh(y-0.2, ad_realloc, w, color='green')
    plt.yticks(y, names);
    plt.title('WebAssembly Random Actions Performance (Higher is Better)')
    plt.xlabel('Average Actions per Microsecond')
    plt.legend(["Alloc + Dealloc", "Alloc + Dealloc + Realloc"])
    plt.tight_layout()
    plt.savefig("{{wasm_perf_plot}}")
    plt.show()
