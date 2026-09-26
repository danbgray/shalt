/** Ingredient packets and Amazon Fresh referral order links. */

import {
  db,
  ensureUser,
  nextId,
  registerPacket,
  resolvePacket,
  resolveRecipe,
} from './store.js';
import {
  createRecipe,
  getRecipe,
  setRecipeIngredients,
  setVisibility,
} from './recipes.js';

function ensureRecipe(ref) {
  const r = resolveRecipe(ref);
  if (!r) {
    throw new Error(
      `Recipe not found: ${typeof ref === 'object' ? ref?.title ?? ref?.id : ref}`
    );
  }
  if (!Array.isArray(r.ingredients)) {
    r.ingredients = [];
  }
  return r;
}

function normalizeItem(ing) {
  if (ing == null) return null;
  if (typeof ing === 'string') {
    const raw = ing.trim();
    if (!raw) return null;
    return { name: raw, quantity: undefined };
  }
  if (typeof ing !== 'object') return null;
  const name = String(ing.name ?? ing.ingredient ?? ing.raw ?? '').trim();
  if (!name) return null;
  const quantity =
    ing.quantity != null
      ? String(ing.quantity).trim()
      : ing.qty != null
        ? String(ing.qty).trim()
        : undefined;
  return { name, quantity: quantity || undefined };
}

/**
 * Create a packet from a recipe's ingredients.
 * @param {object|string} recipeRef
 * @param {string} name
 * @param {string[]|null|undefined} ingredientNames - if null/empty/'all', use all ingredients
 */
export function createPacket(recipeRef, name, ingredientNames) {
  const recipe = ensureRecipe(recipeRef);
  const packetName = String(name ?? '').trim();
  if (!packetName) throw new Error('Packet name is required');

  let source = Array.isArray(recipe.ingredients)
    ? recipe.ingredients.filter((x) => x != null)
    : [];

  const filterNames = Array.isArray(ingredientNames)
    ? ingredientNames
        .map((n) => String(n).trim().toLowerCase())
        .filter(Boolean)
    : [];

  if (filterNames.length > 0) {
    const wanted = new Set(filterNames);
    source = source.filter((ing) => {
      const item = normalizeItem(ing);
      if (!item) return false;
      const n = item.name.toLowerCase();
      if (wanted.has(n)) return true;
      for (const w of wanted) {
        if (n.includes(w) || w.includes(n)) return true;
      }
      return false;
    });
  }

  const items = [];
  for (const ing of source) {
    const item = normalizeItem(ing);
    if (item && item.name) items.push(item);
  }

  const packet = {
    id: nextId('packet'),
    name: packetName,
    recipeId: recipe.id,
    recipeTitle: recipe.title,
    items,
  };
  registerPacket(packet);
  return packet;
}

export function getPacket(name) {
  return resolvePacket(name);
}

export function packetItemCount(packetRef) {
  const packet = resolvePacket(packetRef);
  if (!packet) return 0;
  return Array.isArray(packet.items) ? packet.items.length : 0;
}

export function packetRecipeTitle(packetRef) {
  const packet = resolvePacket(packetRef);
  if (!packet) return null;
  if (packet.recipeTitle) return packet.recipeTitle;
  const recipe = resolveRecipe(packet.recipeId);
  return recipe ? recipe.title : null;
}

export function setAssociateTag(user, tag) {
  const u = ensureUser(user);
  const t = tag == null ? null : String(tag).trim();
  u.associateTag = t || null;
  if (t) {
    db.associateTags.set(u.email, t);
  } else {
    db.associateTags.delete(u.email);
  }
}

export function clearAssociateTag(user) {
  const u = ensureUser(user);
  u.associateTag = null;
  db.associateTags.delete(u.email);
}

function authorOfPacket(packet) {
  const recipe =
    resolveRecipe(packet.recipeId) || resolveRecipe(packet.recipeTitle);
  if (!recipe) return null;
  return ensureUser(recipe.ownerEmail);
}

/**
 * Build an Amazon Fresh order URL for a packet.
 * Includes associate tag when the author has one.
 */
export function amazonFreshOrderLink(packetRef, _viewer) {
  const packet = resolvePacket(packetRef);
  if (!packet) throw new Error('Packet not found');

  const author = authorOfPacket(packet);
  const tag =
    (author && (author.associateTag || db.associateTags.get(author.email))) ||
    null;

  const names = (Array.isArray(packet.items) ? packet.items : [])
    .map((it) => (it && it.name ? String(it.name).trim() : ''))
    .filter(Boolean);

  const params = new URLSearchParams();
  if (names.length) {
    params.set('k', names.join(','));
    names.forEach((n, i) => {
      params.set(`ingredient${i + 1}`, n);
    });
  }
  if (tag) {
    params.set('tag', tag);
  }

  const qs = params.toString();
  const url = qs
    ? `https://www.amazon.com/fresh?${qs}`
    : 'https://www.amazon.com/fresh';

  return { url, href: url };
}

/**
 * Helper: ensure a public recipe exists with given ingredient rows.
 */
export function ensureRecipeWithIngredients(
  title,
  rows,
  ownerEmail = 'maya@example.com'
) {
  let recipe = getRecipe(title);
  if (!recipe) {
    recipe = createRecipe(ownerEmail, title);
  }
  setVisibility(recipe, 'public');
  if (Array.isArray(rows)) {
    setRecipeIngredients(
      recipe,
      rows.map((r) => ({
        name: r.name ?? r[0],
        quantity: r.quantity ?? r[1],
      }))
    );
  }
  return recipe;
}
