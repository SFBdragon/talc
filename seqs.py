import math


x = 4 # log2_start_pow | 16
d = 2 # log2_div_count | 4

for g in range(96):
    #if g % (1<<d) == 0 or g % (1<<d) == (1<<d>>1): print()
    size = ((1 << d) + (g & ((1<<d)-1))) << ((g >> d) + (x-d))

    sl2 = math.floor(math.log2(size))
    g_calc = ((size >> sl2 - d) ^ (1 << d)) + ((sl2 - x) << d)
    # print("g", g, "g_calc", g_calc)
    # print(size >> sl2 - d, (size >> sl2 - d) ^ (1 << d), sl2 - x << d, sl2, x)

    assert g == g_calc

    print("{0:>8} {0:>20b} | ".format(size), end='\n')

# 16 24 32 40 48 56 64
