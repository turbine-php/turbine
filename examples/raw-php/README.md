# Raw PHP Examples

These examples demonstrate Turbine features using plain PHP — no frameworks required.

| Example | Features Demonstrated |
|---------|----------------------|
| [hello-world](hello-world/) | Basic request handling, HTML output, query parameters |
| [rest-api](rest-api/) | JSON API, routing, CORS, compression |
| [session-auth](session-auth/) | Sessions, login/logout, session regeneration, rate limiting |
| [file-upload](file-upload/) | File uploads, sandbox protections, blocked extensions |
| [database-crud](database-crud/) | PDO/SQLite, prepared statements, pagination, SQL guard |
| [websocket-sse](websocket-sse/) | Server-Sent Events, streaming, long-lived connections |

## Running

```bash
cd <example-directory>
turbine --root .
# Open http://127.0.0.1:8080
```
