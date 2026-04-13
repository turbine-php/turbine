<?php
// Verification test: 50KB payload where EVERY response is unique.
// Generates 50KB of random data seeded by hrtime — no two responses match.
// Equivalent workload to random_50k.php but with uniqueness guarantee.

header('Content-Type: application/octet-stream');
header('Cache-Control: no-store');

// Use unique seed per request — hrtime(true) gives nanosecond precision
$seed = hrtime(true) ^ getmypid();
$rng  = new \Random\Randomizer(new \Random\Engine\Xoshiro256StarStar($seed));

for ($i = 0; $i < 50; $i++) {
    echo $rng->getBytesFromString(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
        1023
    ), "\n";
}
