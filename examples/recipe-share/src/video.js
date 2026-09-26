/** Full-recipe video and per-step timestamps. */

import { resolveRecipe } from './store.js';

function ensureRecipe(ref) {
  const r = resolveRecipe(ref);
  if (!r) {
    throw new Error(
      `Recipe not found: ${typeof ref === 'object' ? ref?.title ?? ref?.id : ref}`
    );
  }
  if (!Array.isArray(r.steps)) r.steps = [];
  return r;
}

/**
 * Parse "HH:MM:SS" or "MM:SS" or "SS" into total seconds.
 */
export function parseTimestamp(ts) {
  if (ts == null) return null;
  if (typeof ts === 'number' && Number.isFinite(ts)) return ts;
  const s = String(ts).trim();
  if (!s) return null;
  const parts = s.split(':').map((p) => p.trim());
  if (parts.some((p) => p === '' || Number.isNaN(Number(p)))) {
    throw new Error(`Invalid timestamp: ${ts}`);
  }
  const nums = parts.map((p) => Number(p));
  let seconds = 0;
  if (nums.length === 1) {
    seconds = nums[0];
  } else if (nums.length === 2) {
    seconds = nums[0] * 60 + nums[1];
  } else if (nums.length === 3) {
    seconds = nums[0] * 3600 + nums[1] * 60 + nums[2];
  } else {
    throw new Error(`Invalid timestamp: ${ts}`);
  }
  return seconds;
}

function formatTimestamp(totalSeconds) {
  if (totalSeconds == null || !Number.isFinite(totalSeconds)) return null;
  const s = Math.max(0, Math.floor(totalSeconds));
  const hh = Math.floor(s / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const pad = (n) => String(n).padStart(2, '0');
  return `${pad(hh)}:${pad(mm)}:${pad(ss)}`;
}

export function attachFullVideo(recipeRef, url) {
  const recipe = ensureRecipe(recipeRef);
  const u = String(url || '').trim();
  if (!u) throw new Error('Video URL is required');
  recipe.fullVideoUrl = u;
  return recipe;
}

export function fullVideoUrl(recipeRef) {
  const recipe = resolveRecipe(recipeRef);
  return recipe ? recipe.fullVideoUrl || null : null;
}

function findStep(recipe, number) {
  const n = Number(number);
  let step = recipe.steps.find((s) => s.number === n);
  if (!step) {
    // Auto-create a placeholder step so tagging can proceed when steps
    // were only described in the scenario without explicit addStep.
    step = { number: n, text: '', timestamp: null };
    recipe.steps.push(step);
    recipe.steps.sort((a, b) => a.number - b.number);
  }
  return step;
}

/**
 * Tag a step with a timestamp on the full video.
 * Rejects out-of-order timestamps relative to other tagged steps.
 */
export function tagStepTimestamp(recipeRef, stepNumber, timestamp) {
  const recipe = ensureRecipe(recipeRef);
  if (!recipe.fullVideoUrl) {
    throw new Error('Recipe has no full video');
  }
  const seconds = parseTimestamp(timestamp);
  if (seconds == null) {
    throw new Error('Timestamp is required');
  }
  const formatted = formatTimestamp(seconds);
  const n = Number(stepNumber);

  // Validate order against other steps that already have timestamps
  for (const other of recipe.steps) {
    if (other.number === n) continue;
    if (other.timestamp == null) continue;
    const otherSec = parseTimestamp(other.timestamp);
    if (otherSec == null) continue;
    if (other.number < n && otherSec > seconds) {
      const err = new Error('Step timestamps must be in order');
      err.code = 'TIMESTAMP_ORDER';
      throw err;
    }
    if (other.number > n && otherSec < seconds) {
      const err = new Error('Step timestamps must be in order');
      err.code = 'TIMESTAMP_ORDER';
      throw err;
    }
  }

  const step = findStep(recipe, n);
  step.timestamp = formatted;
  step.timestampSeconds = seconds;
  return {
    step,
    timestamp: formatted,
    seconds,
    ok: true,
  };
}

export function stepTimestamp(recipeRef, stepNumber) {
  const recipe = resolveRecipe(recipeRef);
  if (!recipe || !Array.isArray(recipe.steps)) return null;
  const n = Number(stepNumber);
  const step = recipe.steps.find((s) => s.number === n);
  if (!step || step.timestamp == null) return null;
  return step.timestamp;
}

/**
 * Open a clip for a step: playback starts at the tagged timestamp.
 */
export function openStepClip(recipeRef, stepNumber) {
  const recipe = ensureRecipe(recipeRef);
  const n = Number(stepNumber);
  const step = recipe.steps.find((s) => s.number === n);
  const ts = step && step.timestamp != null ? step.timestamp : null;
  const url = recipe.fullVideoUrl || null;
  return {
    startAt: ts,
    timestamp: ts,
    playbackStartsAt: ts,
    startsAt: ts,
    videoUrl: url,
    url,
    fullVideoUrl: url,
    stepNumber: n,
  };
}
