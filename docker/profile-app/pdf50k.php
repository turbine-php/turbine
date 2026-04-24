<?php
header('Content-Type: application/pdf');
$chunk = str_repeat('x', 1023) . "\n";
for ($i = 0; $i < 50; $i++) { echo $chunk; }
