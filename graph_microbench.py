import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
import os

BENCHMARK_RESULTS_DIR = 'benchmark_results/micro/'
BENCHMARK_RESULT_GRAPHS_DIR = 'benchmark_result_graphs/'

def get_benchmark_data(filename):
    print("reading", filename)
    with open(filename, 'r') as f:
        rows = f.readlines()
    
    allocators = {}
    for row in rows:
        lst = row.strip().split(',')
        if lst[1] != "":
            allocators[lst[0]] = [float(i) for i in lst[1:]]
    return allocators

def plot_data():
    fig, axs = plt.subplots(nrows=1, ncols=2, figsize=(14, 6))

    plot_count = 0
    for filename in os.listdir(BENCHMARK_RESULTS_DIR):
        filepath = BENCHMARK_RESULTS_DIR + filename
        if not os.path.isfile(filepath):
            continue

        allocators = get_benchmark_data(filepath)
        if len(allocators) == 0:
            continue

        print("plotting", filename)

        ax = axs[plot_count]

        ax.boxplot([allocators[x] for x in allocators], 
            sym="", vert=False, showmeans=True, meanline=False, whis=(5, 95), 
            meanprops=dict(marker="D", markerfacecolor="black", markeredgecolor="black"))

        ax.set_title(filename.split(".")[0].title())
        ax.set_xlim(left=0)
        ax.set_yticks(range(1, len(allocators) + 1))
        ax.set_yticklabels([x for x in allocators], rotation=45, ha="right")
        ax.set_xlabel("Ticks")

        legend_elements = [
            Line2D([0], [0], color="orange", lw=1, label="median"),
            Line2D([0], [0], marker="D", color="white", markerfacecolor="black", markeredgecolor="black", label="average")
        ]
        ax.legend(handles=legend_elements, loc="upper right")

        plot_count += 1

    fig.tight_layout()
    plt.show()


def main():
    if not os.path.exists(BENCHMARK_RESULTS_DIR):
        os.mkdir(BENCHMARK_RESULTS_DIR)
    plot_data()

    print("complete")

if __name__ == '__main__':
    main()

