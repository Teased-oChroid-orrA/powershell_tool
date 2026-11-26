# NASA damaged helical-insert shear-out benchmark

Offline validation data only. Source: David L. Johnson and Troy C. Stratton, [*Method for Estimating Thread Strength Reduction of Damaged Parent Holes with Inserts*](https://ntrs.nasa.gov/api/citations/20050180634/downloads/20050180634.pdf), Tables 2 and 3, printed pp. 10-11 (PDF pp. 10-11). Values below were transcribed from rendered page images, including printed averages and empty cells. Table 2 loads are in lbf. Table 3 equivalent missing threads are dimensionless. The study used 7075-T7351 parent material and 15 engaged threads (0.83 in); the baseline equation uses 59,760 lbf / 15 = 3,984 lbf per thread.

`A-R` is the restrained pristine series; `A-U` is the unrestrained pristine series. `-` means no specimen value was printed. Descriptions retain the source's series meaning.

## Table 2: ultimate shear-out load (lbf)

| Series | Description | #1 | #2 | #3 | #4 | #5 | Printed mean |
|---|---|---:|---:|---:|---:|---:|---:|
| A-R | Pristine; top and bottom helical insert retention | 59830 | 59120 | 59980 | 59480 | 60430 | 59768 |
| A-U | Pristine; unrestrained top and bottom helical insert | 55550 | - | 57390 | 57900 | 56960 | 56950 |
| B1 | One bottom thread gone | 52600 | 53370 | 53900 | 52000 | 52710 | 52916 |
| B2 | Two bottom threads gone | 49670 | 49910 | 49740 | 49320 | 49740 | 49676 |
| B3 | Three bottom threads gone | 46170 | 45860 | 46160 | 45650 | 45390 | 45846 |
| C | Oversized hole with standard helical insert | 54110 | 57230 | 56650 | 54300 | 54590 | 55376 |
| D | Bottom three threads oversized | 55760 | 58120 | 55090 | 55020 | 54580 | 55714 |
| E | Small top tapered holes | 56690 | 59030 | 57060 | 56540 | 60940 | 58052 |
| F | Small top tapered holes and center bulge hole | 57560 | 58150 | 57780 | 58540 | 57070 | 57820 |
| G | Small top and bottom tapered holes | 58970 | 59410 | 59630 | 59190 | 59930 | 59426 |
| H | Small top taper and 2/3-down bulge | 58540 | 59050 | 58970 | 58460 | 59190 | 58842 |
| J | Large top tapered holes | 60000 | 59120 | 59850 | 59850 | 60810 | 59926 |
| K | Large top-tapered holes and center bulge hole | 57880 | 56040 | 55090 | 57950 | 56780 | 56748 |
| L | Large top and bottom tapered holes | 58900 | 59050 | 59410 | 53990 | 55380 | 57346 |
| M | Large top taper and 2/3-down bulge | 58970 | 58320 | 53850 | 58610 | 58610 | 57672 |
| N | Pristine hole; two bottom Heli-Coil threads cut | 51720 | 50250 | 51720 | 49820 | 52010 | 51104 |
| P | Oversized parent hole with oversized helical insert | - | 55590 | 55350 | 55350 | 54960 | 55313 |
| Q | Twinsert helical inserts | 56910 | 56960 | 56420 | 56380 | 57110 | 56756 |

## Table 3: equivalent missing threads (printed values)

| Series | #1 | #2 | #3 | #4 | #5 | Printed mean |
|---|---:|---:|---:|---:|---:|---:|
| A-R | 0 | 0 | 0 | 0 | 0 | 0 |
| A-U | 1.04 | - | 0.58 | 0.45 | 0.69 | 0.69 |
| B1 | 1.78 | 1.59 | 1.46 | 1.93 | 1.76 | 1.70 |
| B2 | 3.40 | 3.48 | 3.40 | 3.53 | 3.60 | 3.48 |
| B3 | 2.52 | 2.46 | 2.50 | 2.61 | 2.50 | 2.52 |
| C | 1.40 | 0.62 | 0.77 | 1.36 | 1.28 | 1.09 |
| D | 0.99 | 0.40 | 1.16 | 1.18 | 1.29 | 1.00 |
| E | 0.76 | 0.17 | 0.66 | 0.79 | -0.31 | 0.41 |
| F | 0.54 | 0.39 | 0.48 | 0.29 | 0.66 | 0.47 |
| G | 0.18 | 0.07 | 0.02 | 0.13 | -0.06 | 0.07 |
| H | 0.29 | 0.16 | 0.18 | 0.31 | 0.13 | 0.22 |
| J | -0.08 | 0.15 | -0.04 | -0.04 | -0.28 | -0.06 |
| K | 0.46 | 0.92 | 1.16 | 0.44 | 0.73 | 0.74 |
| L | 0.20 | 0.16 | 0.07 | 1.43 | 1.09 | 0.59 |
| M | 0.18 | 0.35 | 1.47 | 0.27 | 0.27 | 0.51 |
| N | 2.01 | 2.37 | 2.01 | 2.48 | 1.93 | 2.16 |
| P | - | 1.03 | 1.09 | 1.09 | 1.19 | 1.10 |
| Q | 0.70 | 0.69 | 0.82 | 0.83 | 0.65 | 0.74 |

## Source checks and limits

- The printed restrained mean in Table 2 is **59,768 lbf**; the paper's baseline equation rounds it to **59,760 lbf**. Keep both labels distinct.
- Table 3's B2 and B3 values appear exchanged: Table 2 means imply `(59760 - 49676) / 3984 = 2.53` for B2 and `(59760 - 45846) / 3984 = 3.49` for B3, while Table 3 prints 3.48 and 2.52. The sample rows show the same reversal. Do not silently swap or use those two printed equivalent-thread rows as calibration targets.
- Table 3 prints H's mean as 0.22; the mean of its displayed two-decimal specimen values is 0.214, which rounds to 0.21. The printed mean is preserved.
- Table 2's P series has four specimens; its printed mean rounds the four-value arithmetic mean of 55,312.5 lbf to 55,313 lbf.
- These coil-insert tests are a benchmark only for a matched specimen and fixture. They are not tapped-hole or Keensert coefficients, design allowables, or runtime strength corrections.
