/** In-memory store shared across domain modules. */

const state = {
  users: new Map(),
  recipes: new Map(),
  recipesBySlug: new Map(),
  recipesByTitle: new Map(),
  packets: new Map(),
  packetsByName: new Map(),
  patrons: new Map(),
  patronageOffers: new Map(),
  associateTags: new Map(),
  idSeq: 0,
};

export const db = state;

export function resetStore() {
  state.users.clear();
  state.recipes.clear();
  state.recipesBySlug.clear();
  state.recipesByTitle.clear();
  state.packets.clear();
  state.packetsByName.clear();
  state.patrons.clear();
  state.patronageOffers.clear();
  state.associateTags.clear();
  state.idSeq = 0;
}

export function nextId(prefix = 'id') {
  state.idSeq += 1;
  return `${prefix}-${state.idSeq}`;
}

export function slugify(title) {
  return String(title || '')
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

export function userEmail(user) {
  if (user == null) return null;
  if (typeof user === 'string') return user.trim().toLowerCase();
  if (typeof user === 'object') {
    if (user.email) return String(user.email).trim().toLowerCase();
    if (user.userEmail) return String(user.userEmail).trim().toLowerCase();
  }
  return null;
}

export function ensureUser(userOrEmail) {
  const email = userEmail(userOrEmail) || String(userOrEmail || '').trim().toLowerCase();
  if (!email) {
    throw new Error('User email is required');
  }
  let user = state.users.get(email);
  if (!user) {
    user = {
      email,
      associateTag: null,
    };
    state.users.set(email, user);
  }
  return user;
}

export function registerRecipe(recipe) {
  state.recipes.set(recipe.id, recipe);
  if (recipe.slug) {
    state.recipesBySlug.set(recipe.slug, recipe);
  }
  if (recipe.title) {
    state.recipesByTitle.set(String(recipe.title).toLowerCase(), recipe);
  }
  return recipe;
}

export function resolveRecipe(ref) {
  if (ref == null) return null;
  if (typeof ref === 'object') {
    if (ref.id && state.recipes.has(ref.id)) {
      return state.recipes.get(ref.id);
    }
    if (ref.title) {
      const byTitle = state.recipesByTitle.get(String(ref.title).toLowerCase());
      if (byTitle) return byTitle;
    }
    if (ref.slug) {
      const bySlug = state.recipesBySlug.get(ref.slug);
      if (bySlug) return bySlug;
    }
    // Already a recipe-like object living in memory
    if (Array.isArray(ref.ingredients) || Array.isArray(ref.steps)) {
      return ref;
    }
    return null;
  }

  const s = String(ref).trim();
  if (!s) return null;

  // Share path: /r/slug or r/slug
  const pathMatch = s.match(/^\/?r\/([a-z0-9-]+)$/i);
  if (pathMatch) {
    return state.recipesBySlug.get(pathMatch[1].toLowerCase()) || null;
  }

  if (state.recipes.has(s)) return state.recipes.get(s);

  const bySlug = state.recipesBySlug.get(s.toLowerCase());
  if (bySlug) return bySlug;

  const byTitle = state.recipesByTitle.get(s.toLowerCase());
  if (byTitle) return byTitle;

  return null;
}

export function registerPacket(packet) {
  state.packets.set(packet.id, packet);
  if (packet.name) {
    state.packetsByName.set(String(packet.name).toLowerCase(), packet);
  }
  return packet;
}

export function resolvePacket(ref) {
  if (ref == null) return null;
  if (typeof ref === 'object') {
    if (ref.id && state.packets.has(ref.id)) {
      return state.packets.get(ref.id);
    }
    if (ref.name) {
      const byName = state.packetsByName.get(String(ref.name).toLowerCase());
      if (byName) return byName;
    }
    if (Array.isArray(ref.items)) return ref;
    return null;
  }
  const s = String(ref).trim();
  if (!s) return null;
  if (state.packets.has(s)) return state.packets.get(s);
  return state.packetsByName.get(s.toLowerCase()) || null;
}

export function patronKey(patronEmail, authorEmail) {
  return `${String(patronEmail).toLowerCase()}::${String(authorEmail).toLowerCase()}`;
}
