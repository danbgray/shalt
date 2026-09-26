/** Author patronage offers, subscriptions, and patron-only recipe access. */

import {
  db,
  ensureUser,
  patronKey,
  resolveRecipe,
  userEmail,
} from './store.js';
import { ingredientsOf, stepsInOrder } from './recipes.js';

export function enablePatronage(author, amountPerMonth, currency = 'USD') {
  const email = userEmail(author) || String(author || '').trim().toLowerCase();
  ensureUser(email);
  const amount = Number(amountPerMonth);
  if (!Number.isFinite(amount) || amount <= 0) {
    throw new Error('Patronage amount must be a positive number');
  }
  const offer = {
    authorEmail: email,
    amountPerMonth: amount,
    currency: String(currency || 'USD'),
    enabled: true,
  };
  db.patronageOffers.set(email, offer);
  return offer;
}

export function patronageOffer(author) {
  const email = userEmail(author) || String(author || '').trim().toLowerCase();
  return db.patronageOffers.get(email) || null;
}

export function subscribePatron(patron, author, amountPerMonth) {
  const patronEmail =
    userEmail(patron) || String(patron || '').trim().toLowerCase();
  const authorEmail =
    userEmail(author) || String(author || '').trim().toLowerCase();
  ensureUser(patronEmail);
  ensureUser(authorEmail);

  const offer = db.patronageOffers.get(authorEmail);
  if (!offer || !offer.enabled) {
    throw new Error('Author has not enabled patronage');
  }

  const amount =
    amountPerMonth != null ? Number(amountPerMonth) : offer.amountPerMonth;

  const key = patronKey(patronEmail, authorEmail);
  const record = {
    patronEmail,
    authorEmail,
    amountPerMonth: amount,
    active: true,
    cancelled: false,
  };
  db.patrons.set(key, record);
  return record;
}

export function cancelPatronage(patron, author) {
  const patronEmail =
    userEmail(patron) || String(patron || '').trim().toLowerCase();
  const authorEmail =
    userEmail(author) || String(author || '').trim().toLowerCase();
  const key = patronKey(patronEmail, authorEmail);
  const record = db.patrons.get(key);
  if (record) {
    record.active = false;
    record.cancelled = true;
  }
  return record || null;
}

export function isActivePatron(patron, author) {
  const patronEmail =
    userEmail(patron) || String(patron || '').trim().toLowerCase();
  const authorEmail =
    userEmail(author) || String(author || '').trim().toLowerCase();
  const key = patronKey(patronEmail, authorEmail);
  const record = db.patrons.get(key);
  return !!(record && record.active && !record.cancelled);
}

export function activePatronCount(author) {
  const authorEmail =
    userEmail(author) || String(author || '').trim().toLowerCase();
  let count = 0;
  for (const record of db.patrons.values()) {
    if (
      record.authorEmail === authorEmail &&
      record.active &&
      !record.cancelled
    ) {
      count += 1;
    }
  }
  return count;
}

export function setPatronOnly(recipeRef, flag = true) {
  const recipe = resolveRecipe(recipeRef);
  if (!recipe) {
    throw new Error('Recipe not found');
  }
  recipe.patronOnly = !!flag;
  // Patron-only recipes are still "public" in the sense of having a share URL,
  // but gated; mark published so the slug resolves.
  if (recipe.patronOnly) {
    recipe.published = true;
    recipe.visibility = 'public';
  }
  return recipe;
}

/**
 * View a recipe as a given viewer (email or null for anonymous).
 * Returns status 'ok' | 'paywall' | 'not-found'.
 */
export function viewRecipe(recipeRef, viewer) {
  const recipe = resolveRecipe(recipeRef);
  if (!recipe) {
    return {
      status: 'not-found',
      response: 'not-found',
      notFound: true,
      paywall: false,
      title: null,
      ingredients: null,
      steps: null,
    };
  }

  const viewerEmail = viewer == null ? null : userEmail(viewer) || String(viewer).trim().toLowerCase();

  // Private unpublished recipes are not visible except to owner
  if (!recipe.published && recipe.visibility !== 'public') {
    if (!viewerEmail || viewerEmail !== recipe.ownerEmail) {
      return {
        status: 'not-found',
        response: 'not-found',
        notFound: true,
        paywall: false,
        title: null,
        ingredients: null,
        steps: null,
      };
    }
  }

  if (recipe.patronOnly) {
    const isOwner = viewerEmail && viewerEmail === recipe.ownerEmail;
    const patron = viewerEmail
      ? isActivePatron(viewerEmail, recipe.ownerEmail)
      : false;
    if (!isOwner && !patron) {
      return {
        status: 'paywall',
        response: 'paywall',
        notFound: false,
        paywall: true,
        title: recipe.title,
        ingredients: null,
        steps: null,
        recipe: null,
      };
    }
  }

  return {
    status: 'ok',
    response: 'ok',
    notFound: false,
    paywall: false,
    title: recipe.title,
    ingredients: ingredientsOf(recipe),
    steps: stepsInOrder(recipe),
    recipe,
  };
}
