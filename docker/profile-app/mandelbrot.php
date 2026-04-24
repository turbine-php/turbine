<?php
function mandelbrot(int $iters = 300): int {
    $sum = 0;
    for ($y = -1.0; $y < 1.0; $y += 0.1) {
        for ($x = -2.0; $x < 1.0; $x += 0.08) {
            $cr = $x; $ci = $y;
            $zr = 0.0; $zi = 0.0;
            $i = 0;
            while ($i < $iters && ($zr * $zr + $zi * $zi) < 4.0) {
                $tmp = $zr * $zr - $zi * $zi + $cr;
                $zi = 2 * $zr * $zi + $ci;
                $zr = $tmp;
                $i++;
            }
            $sum += $i;
        }
    }
    return $sum;
}
header('Content-Type: text/plain');
echo mandelbrot();
