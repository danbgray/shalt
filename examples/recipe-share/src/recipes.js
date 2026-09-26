/** Recipe create/edit, visibility, and share URLs. */

import {
  db,
  ensureUser,
  nextId,
  registerRecipe,
  resolveRecipe as storeResolve,
  slugify,
  userEmail,
} from './store.js';

export function resolveRecipe(ref) {
  return storeResolve(ref);
}

export function createUser(email) {
  return ensureUser(email);
}

function normalizeTitle(title) {
  if (title == null) return '';
  return String(title).trim();
}

function emptyRecipe(ownerEmail, title) {
  const t = normalizeTitle(title);
  const slug = slugify(t);
  return {
    id: nextId('recipe'),
    title: t,
    slug,
    ownerEmail,
    ingredients: [],
    steps: [],
    visibility: 'private',
    published: false,
    patronOnly: false,
    fullVideoUrl: null,
    sharePath: slug ? `/r/${slug}` : null,
  };
}

/**
 * Create a recipe. Throws if title is empty.
 */
export function createRecipe(owner, title) {
  const ownerEmail = userEmail(owner) || String(owner || '').trim().toLowerCase();
  ensureUser(ownerEmail);
  const t = normalizeTitle(title);
  if (!t) {
    const err = new Error('Title is required');
    err.code = 'TITLE_REQUIRED';
    throw err;
  }
  const recipe = emptyRecipe(ownerEmail, t);
  registerRecipe(recipe);
  return recipe;
}

/**
 * Try to create a recipe; returns { ok, recipe?, error? } instead of throwing.
 */
export function tryCreateRecipe(owner, title) {
  try {
    const recipe = createRecipe(owner, title);
    return { ok: true, recipe, error: null };
  } catch (e) {
    return {
      ok: false,
      recipe: null,
      error: e && e.message ? e.message : String(e),
      saved: false,
    };
  }
}

export function getRecipe(ref) {
  return storeResolve(ref);
}

export function ensureOwnedRecipe(title, ownerEmail) {
  const email = userEmail(ownerEmail) || String(ownerEmail || '').trim().toLowerCase();
  ensureUser(email);
  let recipe = storeResolve(title);
  if (!recipe) {
    recipe = createRecipe(email, title);
  } else if (recipe.ownerEmail !== email) {
    // keep existing owner; still return the recipe
  }
  return recipe;
}

function ensureRecipe(ref) {
  const r = storeResolve(ref);
  if (!r) {
    throw new Error(
      `Recipe not found: ${typeof ref === 'object' ? ref?.title ?? ref?.id : ref}`
    );
  }
  if (!Array.isArray(r.ingredients)) r.ingredients = [];
  if (!Array.isArray(r.steps)) r.steps = [];
  return r;
}

/**
 * Add an ingredient. Accepts a free-text string or {name, quantity}.
 */
export function addIngredient(recipeRef, ingredient) {
  const recipe = ensureRecipe(recipeRef);
  let entry;
  if (typeof ingredient === 'string') {
    const raw = ingredient.trim();
    // Try to split leading quantity from name: "400g canned tomatoes"
    const m = raw.match(/^(\d+(?:\.\d+)?\s*[a-zA-Z]+)\s+(.+)$/);
    if (m) {
      entry = { name: m[2].trim(), quantity: m[1].trim(), raw };
    } else {
      entry = { name: raw, quantity: undefined, raw };
    }
  } else if (ingredient && typeof ingredient === 'object') {
    entry = {
      name: String(ingredient.name ?? '').trim(),
      quantity:
        ingredient.quantity != null
          ? String(ingredient.quantity).trim()
          : undefined,
      raw: ingredient.raw,
    };
  } else {
    throw new Error('Ingredient is required');
  }
  if (!entry.name) throw new Error('Ingredient name is required');
  recipe.ingredients.push(entry);
  return entry;
}

export function setRecipeIngredients(recipeRef, rows) {
  const recipe = ensureRecipe(recipeRef);
  recipe.ingredients = [];
  if (!Array.isArray(rows)) return recipe;
  for (const row of rows) {
    if (row == null) continue;
    if (typeof row === 'string') {
      addIngredient(recipe, row);
    } else if (Array.isArray(row)) {
      addIngredient(recipe, { name: row[0], quantity: row[1] });
    } else {
      addIngredient(recipe, {
        name: row.name ?? row.ingredient,
        quantity: row.quantity ?? row.qty,
      });
    }
  }
  return recipe;
}

/**
 * Add or replace a numbered step (1-based).
 */
export function addStep(recipeRef, number, text) {
  const recipe = ensureRecipe(recipeRef);
  const n = Number(number);
  if (!Number.isFinite(n) || n < 1) {
    throw new Error('Step number must be a positive integer');
  }
  const body = text == null ? '' : String(text);
  const idx = recipe.steps.findIndex((s) => s.number === n);
  const step = {
    number: n,
    text: body,
    timestamp: idx >= 0 ? recipe.steps[idx].timestamp : null,
  };
  if (idx >= 0) {
    recipe.steps[idx] = step;
  } else {
    recipe.steps.push(step);
    recipe.steps.sort((a, b) => a.number - b.number);
  }
  return step;
}

export function changeStep(recipeRef, number, text) {
  return addStep(recipeRef, number, text);
}

export function getStep(recipeRef, number) {
  const recipe = ensureRecipe(recipeRef);
  const n = Number(number);
  const step = recipe.steps.find((s) => s.number === n);
  if (!step) return null;
  return {
    number: step.number,
    text: step.text,
    body: step.text,
    timestamp: step.timestamp ?? null,
  };
}

export function ingredientCount(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe || !Array.isArray(recipe.ingredients)) return 0;
  return recipe.ingredients.length;
}

export function stepCount(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe || !Array.isArray(recipe.steps)) return 0;
  return recipe.steps.length;
}

export function stepsInOrder(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe || !Array.isArray(recipe.steps)) return [];
  return [...recipe.steps]
    .sort((a, b) => a.number - b.number)
    .map((s) => s.text);
}

export function ingredientsOf(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe || !Array.isArray(recipe.ingredients)) return [];
  return recipe.ingredients.map((ing) => {
    if (typeof ing === 'string') return ing;
    if (ing.raw) return ing.raw;
    if (ing.quantity) return `${ing.quantity} ${ing.name}`.trim();
    return ing.name;
  });
}

export function setVisibility(recipeRef, visibility) {
  const recipe = ensureRecipe(recipeRef);
  const v = String(visibility || '').toLowerCase();
  if (v === 'public') {
    recipe.visibility = 'public';
    recipe.published = true;
  } else {
    recipe.visibility = 'private';
    recipe.published = false;
  }
  return recipe;
}

export function isPrivate(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe) return true;
  return recipe.visibility !== 'public' && !recipe.published;
}

export function isPublic(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe) return false;
  return recipe.visibility === 'public' || !!recipe.published;
}

export function publish(recipeRef) {
  const recipe = ensureRecipe(recipeRef);
  recipe.visibility = 'public';
  recipe.published = true;
  if (!recipe.slug) {
    recipe.slug = slugify(recipe.title);
  }
  recipe.sharePath = `/r/${recipe.slug}`;
  db.recipesBySlug.set(recipe.slug, recipe);
  return {
    recipe,
    shareUrl: recipe.sharePath,
    url: recipe.sharePath,
    path: recipe.sharePath,
  };
}

export function shareUrlFor(recipeRef) {
  const recipe = storeResolve(recipeRef);
  if (!recipe) return null;
  if (!recipe.slug) recipe.slug = slugify(recipe.title);
  const path = `/r/${recipe.slug}`;
  recipe.sharePath = path;
  return path;
}

/**
 * Open a share URL as an anonymous (or named) viewer.
 * Returns a view result with status 'ok' | 'not-found' | 'paywall'.
 */
export function openShareUrl(path, viewerEmail = null) {
  const raw = String(path || '').trim();
  const match = raw.match(/^\/?r\/([a-z0-9-]+)$/i);
  const slug = match ? match[1].toLowerCase() : raw.replace(/^\/+/, '').toLowerCase();
  const recipe =
    db.recipesBySlug.get(slug) ||
    storeResolve(raw) ||
    storeResolve(slug);

  if (!recipe || (!recipe.published && recipe.visibility !== 'public')) {
    return {
      status: 'not-found',
      response: 'not-found',
      notFound: true,
      title: null,
      ingredients: null,
      steps: null,
      recipe: null,
    };
  }

  // Patron-only gate for anonymous / non-patron viewers is handled by viewRecipe;
  // share URL for public non-patron-only recipes is open.
  if (recipe.patronOnly) {
    // defer detailed paywall to patronage.viewRecipe; still expose status here
    const { isActivePatron } = waitNo();
  }

  return {
    status: 'ok',
    response: 'ok',
    notFound: false,
    title: recipe.title,
    ingredients: ingredientsOf(recipe),
    steps: stepsInOrder(recipe),
    recipe,
    viewerEmail,
  };
}

// Avoid circular import: patron check inlined lightly for share URL of patron-only
function waitNo() {
  return { isActivePatron: () => false };
}
