<?php

declare(strict_types=1);

/**
 * Turbine — File Upload Example
 *
 * Demonstrates file upload handling with Turbine's sandbox security
 * (blocked extensions, content scanning, open_basedir).
 */

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$uploadDir = __DIR__ . '/../uploads';
$message = '';
$messageType = '';

if (!is_dir($uploadDir)) {
    mkdir($uploadDir, 0755, true);
}

// Handle upload
if ($method === 'POST' && isset($_FILES['file'])) {
    $file = $_FILES['file'];

    if ($file['error'] !== UPLOAD_ERR_OK) {
        $message = 'Upload failed with error code: ' . $file['error'];
        $messageType = 'error';
    } else {
        $filename = basename($file['name']);
        // Sanitise filename
        $safe = preg_replace('/[^a-zA-Z0-9._-]/', '_', $filename);
        $dest = $uploadDir . '/' . $safe;

        if (move_uploaded_file($file['tmp_name'], $dest)) {
            $size = number_format($file['size'] / 1024, 1);
            $message = "Uploaded: {$safe} ({$size} KB)";
            $messageType = 'success';
        } else {
            $message = 'Failed to move uploaded file.';
            $messageType = 'error';
        }
    }
}

// List uploaded files
$files = [];
foreach (glob($uploadDir . '/*') as $f) {
    if (is_file($f)) {
        $files[] = [
            'name' => basename($f),
            'size' => number_format(filesize($f) / 1024, 1) . ' KB',
            'time' => date('Y-m-d H:i', filemtime($f)),
        ];
    }
}

header('Content-Type: text/html; charset=utf-8');
?>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>Turbine — File Upload</title>
    <style>
        body { font-family: system-ui, sans-serif; max-width: 600px; margin: 60px auto; }
        .success { color: #2e7d32; background: #e8f5e9; padding: 10px; border-radius: 4px; }
        .error { color: #c62828; background: #ffebee; padding: 10px; border-radius: 4px; }
        table { width: 100%; border-collapse: collapse; margin-top: 16px; }
        th, td { text-align: left; padding: 8px; border-bottom: 1px solid #ddd; }
        th { background: #f5f5f5; }
        input[type=file] { margin: 8px 0; }
        button { background: #e44d26; color: #fff; border: none; padding: 8px 20px; border-radius: 4px; cursor: pointer; }
    </style>
</head>
<body>
    <h1>File Upload</h1>

    <?php if ($message): ?>
        <p class="<?= $messageType ?>"><?= htmlspecialchars($message, ENT_QUOTES, 'UTF-8') ?></p>
    <?php endif; ?>

    <form method="POST" enctype="multipart/form-data">
        <input type="file" name="file" required>
        <button type="submit">Upload</button>
    </form>

    <p style="color:#888;font-size:13px;">
        Max: <?= ini_get('upload_max_filesize') ?> per file.
        Turbine blocks <code>.php</code>, <code>.phar</code>, etc. by default.
    </p>

    <?php if ($files): ?>
    <h2>Uploaded Files</h2>
    <table>
        <tr><th>Name</th><th>Size</th><th>Date</th></tr>
        <?php foreach ($files as $f): ?>
        <tr>
            <td><?= htmlspecialchars($f['name'], ENT_QUOTES, 'UTF-8') ?></td>
            <td><?= $f['size'] ?></td>
            <td><?= $f['time'] ?></td>
        </tr>
        <?php endforeach; ?>
    </table>
    <?php else: ?>
    <p>No files uploaded yet.</p>
    <?php endif; ?>
</body>
</html>
