import matplotlib.pyplot as plt
import os

BENCHMARK_RESULTS_DIR = 'benchmark_results/'
BENCHMARK_RESULT_GRAPHS_DIR = 'benchmark_graphs/'

def get_benchmark_data(filename):
    with open(filename, 'r') as f:
        rows = f.readlines()

    max_sizes = [int(i) for i in rows[0].split(',')[1:]]

    allocators = {}
    for row in rows[1:]:
        lst = row.split(',')
        allocators[lst[0]] = [float(i) for i in lst[1:]]

    return max_sizes, allocators

def main():
    if not os.path.exists(BENCHMARK_RESULTS_DIR):
        os.mkdir(BENCHMARK_RESULTS_DIR)

    filename = "Random Actions Benchmark.csv"

    max_sizes, data = get_benchmark_data(BENCHMARK_RESULTS_DIR + filename)

    yvalues = []
    for k, v in data.items():
        plt.plot(max_sizes, v, label=k)
        yvalues.append(v)

    plt.xscale('log')
    plt.yscale('log')
    plt.legend()

    plt.title(filename[:filename.find('.csv')])
    plt.xticks(ticks=max_sizes, labels=[str(x) + " / " + str(x*10) for x in max_sizes], rotation=15)
    plt.xlabel('Max Allocation Size (bytes) / Max Reallocation Size (bytes)')
    plt.ylabel('score')

    plt.tight_layout()
    plt.show()

if __name__ == '__main__':
    main()
