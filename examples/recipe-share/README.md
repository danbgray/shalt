# Example: recipe share

Fixture from a real Play run. Not the shalt engine — a small SUT the desk was aimed at.

**Interview:** authors write a recipe (title, ingredients, ordered steps), publish a public link, attach one video with timestamps on steps, group ingredients into packets with an Amazon Fresh order link, and sell patronage at $5/month. Public share text stays free.

| | |
|---|---|
| `spec/` | Feature files with `#observe:` |
| `mockups/journeys/` | Wireframes from design (HTML sketches) |
| `mockups/archive-sketches/` | Earlier sketch pass from the same exercise |
| `steps/` | cucumber-js step defs |
| `src/` | in-memory store the steps call |

Open a wireframe in a browser, e.g. `mockups/journeys/recipes/recipes.html`.

```bash
cd examples/recipe-share
npm install
# then, from a shalt checkout:
# shalt --root examples/recipe-share run
```

This example is a snapshot. Then bodies may still act; treat it as “what Play produced,” not as a gold oracle.
