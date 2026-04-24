<?php
header('Content-Type: application/json');
$out = [
    'sapi'  => PHP_SAPI,
    'zts'   => (bool) PHP_ZTS,
    'jit'   => false,
];
if (function_exists('opcache_get_status')) {
    $s = opcache_get_status(false);
    $out['jit']             = $s['jit']['on']           ?? false;
    $out['jit_buffer_size'] = $s['jit']['buffer_size']  ?? 0;
    $out['jit_buffer_free'] = $s['jit']['buffer_free']  ?? 0;
}
echo json_encode($out, JSON_PRETTY_PRINT);
