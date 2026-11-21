# 28 Sep 2026 — HTTP door, Todo 4/4, Recipe 6/6

Yolo-all on javascript. Write lane 0.6b → 1.7b → 2b-mlx → 4b → 8b. No Grok.

## What worked

- `#observe:` as GET. Todo Thens `fetch(/tasks?filter=…)`. Recipe publish/share/patron `fetch(/r/…)`.
- First colour **red** (404 listener), then Build filled store + server, then green. `never_red` 0.
- cucumber-js: Todo 4/4 14 steps; Recipe 6/6 30 steps.
- sqlite `.shalt/app.sqlite`. Sketch UI under `src/ui/`.

## What broke (fixed in the harness)

1. **`const headers = {}` counted as a stub.** `body_is_stub` used `contains("{}")`. HTTP Thens looked unbound, Play launched 8b, 8b wrote `src/store.js`, zone fail. Fix: only an empty function is a stub.
2. **`this.baseUrl + this.lastPath || fallback`.** JS concatenates before `||`. Port glued to `undefined` → `http://127.0.0.1:59252undefined`. Fix: always parenthesize the path expression. Observe with a literal `/r/slug` must too.
3. **8b stepwright walks out of `steps/`.** Isolation held. Skip the writer when the journey is already bound.
4. **Arial dumps as the product.** Normalize: full `<html>` / Arial / invented rids → spec-rid sketch fragments.
5. **Complete store before first run.** Never-red 4/4 and 6/6. Steps now throw; Build fills the store after red.

## Still thin

- Packet count and video timestamp observe the store, not HTTP.
- Sharing Then lived on `patrons.steps.js` (same phrase). Works, messy.
- Product `node src/server.js` did not restore sqlite or serve `src/ui/`. Boot talked to the store, not GET/POST.

## Next Play

Wipe Todo + Recipe share. Expect red then green, GET `/tasks` and GET `/r/{id}`, UI served from the same server. Verify with Playwright.

## Replay (same day, after stall-seed)

Ollama 8b timed out; Play had re-enqueued Design 50 times. Harness now seeds the sketch when the write lane is exhausted.

Playwright against `node src/server.js`:

- Todo `POST /tasks` → `GET /tasks?filter=active` includes Buy milk; UI add posts Call Sam. **PASS**
- Recipe `POST /recipes` → `GET /r/weeknight-tomato-pasta` HTML title. **PASS**

Ledger: Todo 4/4 and Recipe 6/6, first colour red, `never_red` 0.

## Replay 2 (29 Sep) — full HTTP door + usable `/`

Notes from replay 1 still true. Remaining thin spots that this harness pass closes:

1. **Packet / video / patronage observed the store.** Seed `#observe:` is now GET `/r/{id}` (packets, timestamps) and GET `/patrons?user=`.
2. **GET `/r/{id}` was an h1.** Server now renders ingredients, steps + timestamps, packets + Amazon Fresh URL, video.
3. **`/` was a prototype landing.** Index chrome is the product form (todo add / recipe publish) so Playwright can drive `/`.
4. **sqlite missed packet/video writes.** Mutators dump. `packetsFor` lists packets on the share page.
5. **POST `/recipes` title-only.** Hydrates ingredients, steps, packets, video, patron-only.

Wipe Todo + Recipe share again. Envelope stays paused. Verify with Playwright: todo UI add on `/`; recipe GET `/r/weeknight-tomato-pasta` includes tomatoes, step, timestamp, packet, Amazon link.

### Replay 2 result (29 Sep)

Yolo-all javascript. Envelope paused. Write lane filled oracles; first suite 4 red / 6 red, then Build green. `never_red` 0.

Playwright against `node src/server.js` with a fresh sqlite:

- Todo **PASS**: `POST /tasks` → `GET /tasks?filter=active` includes Buy milk; UI Add on `/` posts Call Sam.
- Recipe **PASS**: `POST /recipes` with ingredients/steps/packet/video → `GET /r/weeknight-tomato-pasta` includes tomatoes, Boil water, 00:00:12, sauce, Amazon Fresh.

cucumber-js: Todo 4/4 14 steps; Recipe 6/6 30 steps.

Harness extras this pass: packet/video/patron `#observe:` is GET; `recipePage` HTML; `/patrons`; index product chrome; browser boot talks HTTP only (`node:sqlite` cannot load in Chrome).
