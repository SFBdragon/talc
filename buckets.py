import math

# modify these parameters to determine the bucketing strategy
# the main things we want are 
# - coverage up to the 100MiB-1GiB area
# - minimize sizes per bucket
# - facilitate particularly quick allocation of small sizes
# - don't sacrifice the speed of large allocations much

## for 32-bit machines:
#word_size = 4
#word_buckets_limit = 64
#double_buckets_limit = 128
#exp_fractions = 2

# for 64-bit machines:
word_size = 8
word_buckets_limit = 256
double_buckets_limit = 512
exp_fractions = 4


# the rest of this 

min_chunk_size = word_size * 3

word_bins_count = (word_buckets_limit - min_chunk_size) // word_size
print("word bins count:", word_bins_count)

for i, sb in enumerate(range(min_chunk_size, word_buckets_limit, word_size)):
    print("{1:>3}: {0:>8} {0:>20b} | ".format(sb, i), end='\n')

double_bins_count = (double_buckets_limit - word_buckets_limit) // (2*word_size)
print("double bins count:", double_bins_count)

for i, bsb in enumerate(range(word_buckets_limit, double_buckets_limit, 2*word_size)):
    print("{1:>3}: {0:>8} {0:>20b} | ".format(bsb, i), end='\n')

print("log bins")

g_ofst = int(math.log2(double_buckets_limit)) # log2_start_pow | 16
g_p2dv = int(math.log2(exp_fractions)) # log2_div_count | 4

for g in range(0, (word_size * 8 * 2) - word_bins_count - double_bins_count):
    # calculation for size from g
    size = ((1 << g_p2dv) + (g & ((1<<g_p2dv)-1))) << ((g >> g_p2dv) + (g_ofst-g_p2dv))

    # calculation of g from size
    size_log2 = math.floor(math.log2(size))
    g_calc = ((size >> size_log2 - g_p2dv) ^ (1<<g_p2dv)) + ((size_log2-g_ofst) << g_p2dv)

    # check that they match
    assert g == g_calc

    print("{1:>3}: {0:>8} {0:>20b} | ".format(size, g + word_bins_count + double_bins_count), end='\n')
