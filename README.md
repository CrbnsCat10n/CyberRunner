# CyberRunner

CyberRunner is a cross-platform replay tool. It provides one binary with two frontends:

- GUI: launch without arguments.
- CLI: launch with `--cli` or a subcommand.

The business logic lives in the core library under `src/` and is shared by both frontends. The CLI driver is `src/main.rs`; the GUI driver is `src/gui.rs`.

## Features

- Fetch health-running venue JSON.
- Generate venue-based running replay packets.
- Preview/save HTTP packet files.
- Query `running/run/count`.
- Send a full replay to a configured server.
- GUI map preview.

You must provide an Authorization token from an authorized session.

## GUI Usage

Start the GUI by running the binary without arguments.

Common workflow:

1. Fill `Authorization` and `loginName`.
2. Load or fetch venue JSON from the map panel.
3. Select a venue.
4. Adjust `trackKm`, `duration`, sex, semester, and optional tuning values.
5. Click `Preview` to inspect generated packets and route.
6. Enable `Send requests to server`.
7. Click `Get Run Count` or `Start Replay`.

GUI notes:

- `Male` maps to `sex = 0` and sets `resultKm = 2.00`.
- `Female` maps to `sex = 1` and sets `resultKm = 1.60`.
- `seed` and `inset m` are under `Track tuning`.
- `resultKm`, `runStatus`, and `standardPace` are under `Runner details`.
- `semesterId` is auto-generated from the selected academic year and term, but remains editable.

## CLI Overview

Show help:

```bash
cyber-runner --help
```

Subcommands:

```text
preview       Generate running/save HTTP packet files
fetch-venues  Fetch venue JSON from a configured server
run-count     Print or send the running/run/count request
replay        Generate and optionally send replay packets
```

If you run:

```bash
cyber-runner --cli
```

it behaves like `preview` with default options.

## Shared Replay Options

These options are used by `preview`, `run-count`, and `replay`:

```text
--venues-json <PATH>        Venue JSON path
--venue-index <N>           Venue index from the venue JSON
--out-dir <PATH>            Output directory for generated packets
--duration-minutes <MIN>    Replay duration
--result-km <TEXT>          Literal resultKm sent in request bodies
--track-km <KM>             Generated route distance
--login-name <TEXT>         loginName
--semester-id <TEXT>        semesterId
--semester-name <TEXT>      semesterName
--sex <0|1>                 0 = male, 1 = female
--run-status <TEXT>         runStatus
--standard-pace <TEXT>      standardPace
--inset-m <METERS>          Track inward offset from venue boundary
--seed <NUMBER>             Deterministic generation seed
--authorization <TOKEN>     Bearer token or token-only value
--user-agent <TEXT>         User-Agent header
--referer <URL>             Referer header
```

## CLI Workflow Example

Prepare shell variables:

```bash
export APP="./target/release/cyber-runner"
export BASE_URL="https://ty.tongji.edu.cn/msports"
export TJ_TOKEN="your_token_here"
export LOGIN_NAME="your_login_name_here"
```

### 1. Authorization

CyberRunner does not perform login. Pass an existing token:

```bash
--authorization "$TJ_TOKEN"
```

You may pass either:

```text
Bearer xxxxx
```

or only:

```text
xxxxx
```

CyberRunner normalizes token-only values to `Bearer xxxxx`.

### 2. Fetch Venues

```bash
$APP fetch-venues \
  --base-url "$BASE_URL" \
  --authorization "$TJ_TOKEN" \
  --token-query \
  --out CyberRunner/output/health_run_venues.json
```

Optional query parameters:

```bash
--longitude 121.498
--latitude 31.280
--open-type 0
```

### 3. Query Run Count

Print the request only:

```bash
$APP run-count \
  --login-name "$LOGIN_NAME" \
  --semester-id 121 \
  --run-status 0 \
  --authorization "$TJ_TOKEN"
```

Send the request:

```bash
$APP run-count \
  --base-url "$BASE_URL" \
  --send \
  --login-name "$LOGIN_NAME" \
  --semester-id 121 \
  --run-status 0 \
  --authorization "$TJ_TOKEN"
```

### 4. Preview Replay Packets

```bash
$APP preview \
  --venues-json CyberRunner/output/health_run_venues.json \
  --venue-index 1 \
  --login-name "$LOGIN_NAME" \
  --authorization "$TJ_TOKEN" \
  --semester-id 121 \
  --semester-name "2025-2026学年第2学期" \
  --sex 0 \
  --result-km 2.00 \
  --track-km 2.0 \
  --duration-minutes 10 \
  --run-status 0 \
  --standard-pace 8.00 \
  --inset-m 15 \
  --seed 20260601
```

Generated files are written under:

```text
CyberRunner/output/replay_packets_YYYYMMDD_HHMMSS/
```

### 5. Send a Full Replay

Only send to services/accounts you are authorized to use.

Male 2 km example:

```bash
$APP replay \
  --send \
  --base-url "$BASE_URL" \
  --venues-json CyberRunner/output/health_run_venues.json \
  --venue-index 1 \
  --login-name "$LOGIN_NAME" \
  --authorization "$TJ_TOKEN" \
  --semester-id 121 \
  --semester-name "2025-2026学年第2学期" \
  --sex 0 \
  --result-km 2.00 \
  --track-km 2.0 \
  --duration-minutes 10 \
  --run-status 0 \
  --standard-pace 8.00 \
  --inset-m 15 \
  --seed 20260601
```

Female 1.6 km example:

```bash
$APP replay \
  --send \
  --base-url "$BASE_URL" \
  --venues-json CyberRunner/output/health_run_venues.json \
  --venue-index 1 \
  --login-name "$LOGIN_NAME" \
  --authorization "$TJ_TOKEN" \
  --semester-id 121 \
  --semester-name "2025-2026学年第2学期" \
  --sex 1 \
  --result-km 1.60 \
  --track-km 1.6 \
  --duration-minutes 10 \
  --run-status 0 \
  --standard-pace 8.00 \
  --inset-m 15 \
  --seed 20260601
```

`replay --send` writes packet files first, then sends them according to their scheduled replay times:

- First running packet is sent almost immediately.
- Later running packets are sent about once per minute.
- The finish packet is sent about 6 seconds after the last running packet.
- If the server returns a `uid`, CyberRunner captures it and signs following packets with that `uid`.

## Semester ID Rule

The GUI computes semester IDs from this known base:

```text
2025-2026 第2学期 = 121
```

Each semester changes the ID by 1:

```text
semesterId = 121 + (start_year - 2025) * 2 + (term - 2)
```

## Notes

- The GUI map uses online CARTO tiles, so map display needs network access.
- The generated request coordinates are not converted for the map; conversion is only used for CARTO display alignment.
- GUI-only behavior includes visual map display, theme-based map source selection, and form conveniences such as sex/semester selectors.
- CLI-only behavior includes writing directly to stdout and shell-friendly automation.

