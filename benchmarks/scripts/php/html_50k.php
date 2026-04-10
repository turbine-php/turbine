<?php
// Test 1: HTML 50KB response (simulates server-side rendered landing page)
header('content-type: text/html; charset=utf-8');

$str = str_repeat('x', 1023) . "\n";

for ($i = 0; $i < 50; $i++) {
    echo $str;
}
