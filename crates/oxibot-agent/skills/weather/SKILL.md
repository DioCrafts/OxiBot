---
name: weather
description: Get current weather and forecasts (no API key required).
metadata: {"nanobot":{"emoji":"🌤️","requires":{"bins":["curl"]}}}
---

# Weather

Two free services, no API keys needed.

## 1. wttr.in (quick, one-liner)

```bash
# Current weather for a city
curl -s "wttr.in/Madrid?format=%C+%t+%h+%w"

# Detailed forecast (3 days)
curl -s "wttr.in/Madrid"

# One-line compact format
curl -s "wttr.in/Madrid?format=3"

# JSON output
curl -s "wttr.in/Madrid?format=j1"
```

### Format codes

| Code | Meaning |
|------|---------|
| `%C` | Condition |
| `%t` | Temperature |
| `%h` | Humidity |
| `%w` | Wind |
| `%p` | Precipitation |

## 2. Open-Meteo (detailed, free API)

```bash
# Get coordinates first, then forecast
curl -s "https://geocoding-api.open-meteo.com/v1/search?name=Madrid&count=1" | jq '.results[0] | {lat: .latitude, lon: .longitude}'

# 7-day forecast
curl -s "https://api.open-meteo.com/v1/forecast?latitude=40.42&longitude=-3.70&daily=temperature_2m_max,temperature_2m_min,precipitation_sum&timezone=auto"
```

## Tips

- For quick checks, prefer `wttr.in` (simpler)
- For data analysis, prefer Open-Meteo (structured JSON)
- Always include the city name from the user's request
