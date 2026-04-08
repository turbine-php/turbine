<?php

declare(strict_types=1);

/**
 * Turbine — Session & Authentication Example
 *
 * Demonstrates Turbine's session handling with a simple login/logout flow.
 * Users: admin/admin123, user/user123
 */

session_start();

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$uri = trim(parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH), '/');

// Simulated user database
$users = [
    'admin' => ['password' => password_hash('admin123', PASSWORD_BCRYPT), 'role' => 'admin'],
    'user'  => ['password' => password_hash('user123', PASSWORD_BCRYPT), 'role' => 'user'],
];

$loggedIn = isset($_SESSION['username']);
$flash = $_SESSION['flash'] ?? null;
unset($_SESSION['flash']);

// Handle POST /login
if ($method === 'POST' && $uri === 'login') {
    $username = trim($_POST['username'] ?? '');
    $password = $_POST['password'] ?? '';

    if (
        isset($users[$username]) &&
        password_verify($password, $users[$username]['password'])
    ) {
        // Regenerate session ID to prevent fixation
        session_regenerate_id(true);
        $_SESSION['username'] = $username;
        $_SESSION['role'] = $users[$username]['role'];
        $_SESSION['login_time'] = time();
        header('Location: /');
        exit;
    }

    $_SESSION['flash'] = 'Invalid username or password.';
    header('Location: /login');
    exit;
}

// Handle GET /logout
if ($uri === 'logout') {
    session_destroy();
    header('Location: /login');
    exit;
}

// Protected area
if ($uri === '' && $loggedIn) {
    header('Content-Type: text/html; charset=utf-8');
    $username = htmlspecialchars($_SESSION['username'], ENT_QUOTES, 'UTF-8');
    $role = htmlspecialchars($_SESSION['role'], ENT_QUOTES, 'UTF-8');
    $loginTime = date('Y-m-d H:i:s', $_SESSION['login_time']);
    echo <<<HTML
    <!DOCTYPE html>
    <html lang="en">
    <head><meta charset="utf-8"><title>Dashboard</title>
    <style>body{font-family:system-ui,sans-serif;max-width:500px;margin:60px auto;}
    .card{border:1px solid #ddd;border-radius:8px;padding:24px;margin:16px 0;}
    a{color:#e44d26;}</style></head>
    <body>
        <h1>Welcome, {$username}!</h1>
        <div class="card">
            <p><strong>Role:</strong> {$role}</p>
            <p><strong>Logged in at:</strong> {$loginTime}</p>
            <p><strong>Session ID:</strong> <code>{$_COOKIE['PHPSESSID']}</code></p>
        </div>
        <a href="/logout">Logout</a>
    </body>
    </html>
    HTML;
    exit;
}

// Redirect unauthenticated users to login
if ($uri === '' && !$loggedIn) {
    header('Location: /login');
    exit;
}

// Login page
if ($uri === 'login') {
    header('Content-Type: text/html; charset=utf-8');
    $error = $flash ? '<p style="color:red;">' . htmlspecialchars($flash, ENT_QUOTES, 'UTF-8') . '</p>' : '';
    echo <<<HTML
    <!DOCTYPE html>
    <html lang="en">
    <head><meta charset="utf-8"><title>Login</title>
    <style>body{font-family:system-ui,sans-serif;max-width:400px;margin:80px auto;}
    input{display:block;width:100%;padding:8px;margin:8px 0;box-sizing:border-box;border:1px solid #ccc;border-radius:4px;}
    button{background:#e44d26;color:#fff;border:none;padding:10px 24px;border-radius:4px;cursor:pointer;width:100%;margin-top:8px;}</style></head>
    <body>
        <h1>Login</h1>
        {$error}
        <form method="POST" action="/login">
            <label>Username</label>
            <input name="username" type="text" required autofocus>
            <label>Password</label>
            <input name="password" type="password" required>
            <button type="submit">Sign In</button>
        </form>
        <p style="color:#888;font-size:13px;margin-top:16px;">Demo users: admin/admin123, user/user123</p>
    </body>
    </html>
    HTML;
    exit;
}

http_response_code(404);
echo '404 Not Found';
