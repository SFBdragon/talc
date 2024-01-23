# The MIT License (MIT)
# 
# Copyright © 2023 Roee Shoshani, Guy Nir
# 
# Permission is hereby granted, free of charge, to any person obtaining a copy 
# of this software and associated documentation files (the “Software”), to deal 
# in the Software without restriction, including without limitation the rights 
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell 
# copies of the Software, and to permit persons to whom the Software is 
# furnished to do so, subject to the following conditions:
# 
# The above copyright notice and this permission notice shall be included 
# in all copies or substantial portions of the Software.
# 
# THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR 
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, 
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE 
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, 
# WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN 
# CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

# Modified by Shaun Beautement
# - extended sample count to 12
# - made Talc the baseline for comparison

import matplotlib.pyplot as plt
import glob
import itertools
import os
from qbstyles import mpl_style

BENCHMARK_RESULTS_DIR = 'benchmark_results/'
BENCHMARK_RESULT_GRAPHS_DIR = 'benchmark_result_graphs/'

mpl_style(True)

def get_benchmark_data(filename):
    with open(filename, 'r') as f:
        rows = f.readlines()
    allocators = {}
    for row in rows:
        lst = row.split(',')
        allocators[lst[0]] = [float(i) for i in lst[1:]]
    return allocators

def plot_benchmark(filename):
    xaxis = [i/10 for i in range(2, 10+1, 2)]
    data = get_benchmark_data(filename)
    yvalues = []
    for k,v in data.items():
        plt.plot(xaxis, v, label=k)
        yvalues.append(v)

    plt.legend()
    test_name = filename[len(BENCHMARK_RESULTS_DIR): filename.find('.csv')]

    if test_name == "random_actions":
        plt.title("Random Actions Benchmark")
    else:
        plt.title(test_name)

    plt.xlabel('time (seconds)\n')
    plt.ylabel('score')
    plt.gca().set_ylim(bottom=0)
    
    full_diff_str = ''
    
    k1 = 'talc'
    v1 = data.pop(k1)
    for k2 in data.keys():
        v2 = data[k2]
        diff = round(difference_average(v1, v2), 2)    
        full_diff_str += f'{k1} - {k2}: {diff}%\n'
            
    plt.figtext(0.14, 0.64, full_diff_str, fontsize=10)
    plt.show()

def percentage_difference(a,b):
    return a / b * 100

def difference_average(plt_a, plt_b):
    differences = []
    for a, b in zip(plt_a, plt_b):
        differences.append(percentage_difference(a,b))
    return sum(differences)/len(differences)

def main():
    if not os.path.exists(BENCHMARK_RESULTS_DIR):
        os.mkdir(BENCHMARK_RESULTS_DIR)
    for filename in glob.glob(BENCHMARK_RESULTS_DIR+'*.*'):
        plot_benchmark(filename)

if __name__ == '__main__':
    main()
    print('done')