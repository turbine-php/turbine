<?php

declare(strict_types=1);

/**
 * Turbine — Hello World Example
 *
 * The simplest possible PHP application running on Turbine.
 * Demonstrates basic request handling and response output.
 */

header('Content-Type: text/html; charset=utf-8');

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$uri = $_SERVER['REQUEST_URI'] ?? '/';
$name = $_GET['name'] ?? 'World';

?>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>Turbine — Hello World</title>
    <style>
        body { font-family: system-ui, sans-serif; max-width: 600px; margin: 80px auto; text-align: center; }
        h1 { color: #e44d26; }
        code { background: #f4f4f4; padding: 2px 8px; border-radius: 4px; }
    </style>
</head>
<body>
    <h1>Hello, <?= htmlspecialchars($name, ENT_QUOTES, 'UTF-8') ?>!</h1>
    <p>Served by <strong>Turbine</strong> via PHP <?= PHP_VERSION ?></p>
    <p>Method: <code><?= $method ?></code> — URI: <code><?= htmlspecialchars($uri, ENT_QUOTES, 'UTF-8') ?></code></p>
    <p>Try: <a href="?name=Turbine">?name=Turbine</a></p>
</body>
</html>
