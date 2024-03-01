import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
import os

BENCHMARK_RESULTS_DIR = 'benchmark_results/micro/'
BENCHMARK_RESULT_GRAPHS_DIR = 'benchmark_result_graphs/'

def get_benchmark_data(filename):
    print("reading", filename)
    with open(filename, 'r') as f:
        rows = f.readlines()
    
    allocators = []
    for row in rows:
        lst = row.strip().split(',')
        if lst[1] != "":
            allocators.append((lst[0], [float(i) for i in lst[1:]]))
    return allocators

def plot_data(filename, filepath):
    allocators = get_benchmark_data(filepath)
    if len(allocators) == 0:
        return
    
    # sort by median
    allocators.sort(key=lambda a: -a[1][2])

    print("plotting", filename)

    rightlim = max([x[1][3] for x in allocators])*1.2

    plt.boxplot([x[1] for x in allocators], sym="", vert=False, showmeans=False, meanline=False, whis=(0, 100))

    i = 1
    for al in allocators:
        plt.annotate(str(int(al[1][4])), (rightlim - 50, i), )
        i += 1

    plt.title(filename.split(".")[0])
    plt.xlim(left=0, right=rightlim)
    plt.yticks(range(1, len(allocators) + 1), [x[0] for x in allocators])
    plt.xlabel("Ticks")

    plt.tight_layout()
    plt.show()


def main():
    if not os.path.exists(BENCHMARK_RESULTS_DIR):
        print("No results dir. Has the benchmark been run?")
        return
    
    for filename in os.listdir(BENCHMARK_RESULTS_DIR):
        filepath = BENCHMARK_RESULTS_DIR + filename
        if not os.path.isfile(filepath):
            continue

        plot_data(filename, filepath)

    print("complete")

if __name__ == '__main__':
    main()

