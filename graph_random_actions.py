import matplotlib.pyplot as plt
import os

BENCHMARK_RESULTS_DIR = 'benchmark_results/'
BENCHMARK_RESULT_GRAPHS_DIR = 'benchmark_graphs/'

def get_benchmark_data(filename):
    with open(filename, 'r') as f:
        rows = f.readlines()
    allocators = {}
    for row in rows:
        lst = row.split(',')
        allocators[lst[0]] = [float(i) for i in lst[1:]]
    return allocators

def main():
    if not os.path.exists(BENCHMARK_RESULTS_DIR):
        os.mkdir(BENCHMARK_RESULTS_DIR)

    filename = "random_actions.csv"

    xaxis = [i/10 for i in range(2, 10+1, 2)]
    data = get_benchmark_data(BENCHMARK_RESULTS_DIR + filename)
    yvalues = []
    for k,v in data.items():
        plt.plot(xaxis, v, label=k)
        yvalues.append(v)

    plt.legend()
    test_name = filename[len(BENCHMARK_RESULTS_DIR): filename.find('.csv')]

    plt.title("Random Actions Benchmark")
    plt.xlabel('time (seconds)\n')
    plt.ylabel('score')
    plt.gca().set_ylim(bottom=0)
    
    plt.show()

if __name__ == '__main__':
    main()
