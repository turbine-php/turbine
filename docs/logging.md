# Structured Logging

Turbine provides a `turbine_log()` PHP function that outputs structured JSON logs, compatible with Datadog, Grafana Loki, Elastic, and other log aggregation tools.

## Configuration

```toml
[structured_logging]
enabled = true
# Output: "stdout", "stderr", or a file path
output = "stderr"
```

## Usage in PHP

```php
<?php
// Basic logging
turbine_log('User logged in', 'info', ['user_id' => 42]);
turbine_log('Payment failed', 'error', ['order_id' => 123, 'reason' => 'insufficient_funds']);
turbine_log('Cache miss', 'debug', ['key' => 'user:42:profile']);

// Log levels
turbine_log('Trace message', 'trace');
turbine_log('Debug details', 'debug');
turbine_log('Informational', 'info');
turbine_log('Warning sign', 'warn');
turbine_log('Error occurred', 'error');
```

## Output Format

Logs are output as JSON, one object per line:

```json
{"timestamp":"2026-04-07T08:30:00Z","level":"info","msg":"User logged in","user_id":42}
{"timestamp":"2026-04-07T08:30:01Z","level":"error","msg":"Payment failed","order_id":123,"reason":"insufficient_funds"}
```

## Integration with Log Aggregators

### Datadog

```toml
[structured_logging]
output = "/var/log/turbine/app.log"
```

Configure the Datadog agent to tail the log file:

```yaml
# /etc/datadog-agent/conf.d/turbine.yaml
logs:
  - type: file
    path: /var/log/turbine/app.log
    service: my-app
    source: turbine
```

### Grafana Loki (via Promtail)

```yaml
# promtail config
scrape_configs:
  - job_name: turbine
    static_configs:
      - targets: [localhost]
        labels:
          job: turbine
          __path__: /var/log/turbine/app.log
```

## Compared to error_log()

| | `turbine_log()` | `error_log()` |
|---|---|---|
| Format | Structured JSON | Plain text |
| Context | Key-value pairs | String only |
| Levels | trace/debug/info/warn/error | Single level |
| Parsing | Machine-readable | Requires regex |

Use `turbine_log()` for application logs that need to be queried and analyzed. Use `error_log()` for simple debugging.
