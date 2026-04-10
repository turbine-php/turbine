<?php
// Test 2: PDF 50KB response (binary content-type, same payload)
header('content-type: application/pdf');

$str = str_repeat('x', 1023) . "\n";

for ($i = 0; $i < 50; $i++) {
    echo $str;
}
