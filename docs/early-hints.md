# Early Hints (HTTP 103)

Turbine natively supports [HTTP 103 Early Hints](https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/103), allowing the browser to preload resources before the PHP response is ready. This can reduce page load times by up to 30%.

## How It Works

```
Client ─── GET /page ──→ Turbine ──→ PHP starts processing
       ←── 103 Early Hints ───       (still processing...)
           Link: </style.css>; rel=preload
           Link: </app.js>; rel=preload
       ←── 200 OK ────────────       (done!)
           <html>...
```

The browser starts downloading CSS and JS while PHP generates the response.

## Configuration

```toml
[early_hints]
enabled = true
```

Enabled by default. No additional configuration needed.

## Usage in PHP

Send early hints using the `header()` function with the `Link` header:

```php
<?php
// Send preload hints before generating the page
header('Link: </css/app.css>; rel=preload; as=style', false);
header('Link: </js/app.js>; rel=preload; as=script', false);

// PHP continues processing...
echo '<html>...</html>';
```

In Laravel Blade:

```php
<?php
header('Link: <' . asset('css/app.css') . '>; rel=preload; as=style', false);
header('Link: <' . Vite::asset('resources/js/app.js') . '>; rel=preload; as=script', false);
?>
@extends('layouts.app')
...
```

## HTTP/2 vs HTTP/1.1

| Protocol | Behavior |
|----------|----------|
| HTTP/2 | Sends 103 informational response frame (true Early Hints) |
| HTTP/1.1 | Includes Link headers in the final 200 response |

For full benefit, use HTTP/2 (enabled automatically with TLS).
