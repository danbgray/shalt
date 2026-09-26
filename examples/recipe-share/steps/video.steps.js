import { Given, When, Then } from '@cucumber/cucumber';
import assert from 'node:assert/strict';

import { addStep, createRecipe, getRecipe } from '../src/recipes.js';
import { attachFullVideo, fullVideoUrl, openStepClip, stepTimestamp, tagStepTimestamp } from '../src/video.js';
When('I attach video {string} as the full recipe video', function (a0) {
  attachFullVideo(this.lastRecipe, a0);
});

Then('the recipe {string} has full video {string}', function (a0, a1) {
  assert.equal(fullVideoUrl(a0), a1);
});

Given('the recipe has full video {string}', function (a0) {
  attachFullVideo(this.lastRecipe, a0);
});

Given('step 1 is {string}', function (a0) {
  addStep(this.lastRecipe, 1, a0);
});

Given('step 2 is {string}', function (a0) {
  addStep(this.lastRecipe, 2, a0);
});

When('I tag step 1 at timestamp {string}', function (a0) {
  tagStepTimestamp(this.lastRecipe, 1, a0);
});

When('I tag step 2 at timestamp {string}', function (a0) {
  tagStepTimestamp(this.lastRecipe, 2, a0);
});

Then('step 1 links to {string} in the full video', function (a0) {
  assert.equal(stepTimestamp(this.lastRecipe, 1), a0);
});

Then('step 2 links to {string} in the full video', function (a0) {
  assert.equal(stepTimestamp(this.lastRecipe, 2), a0);
});

Given('a recipe {string} with full video {string}', function (a0, a1) {
  this.lastRecipe = getRecipe(a0) || createRecipe('maya@example.com', a0);
  attachFullVideo(this.lastRecipe, a1);
});

Given('step 2 is tagged at {string}', function (a0) {
  if (!(this.lastRecipe.steps || []).some((s) => s.number === 2)) addStep(this.lastRecipe, 2, 'step 2');
  tagStepTimestamp(this.lastRecipe, 2, a0);
});

When('a viewer opens the clip for step 2', function () {
  this.lastClip = openStepClip(this.lastRecipe, 2);
});

Then('playback starts at {string} of {string}', function (a0, a1) {
  const ts = this.lastClip && (this.lastClip.startAt || this.lastClip.timestamp || this.lastClip.playbackStartsAt);
  assert.equal(ts, a0);
});

Given('step 1 is tagged at {string}', function (a0) {
  if (!(this.lastRecipe.steps || []).some((s) => s.number === 1)) addStep(this.lastRecipe, 1, 'step 1');
  tagStepTimestamp(this.lastRecipe, 1, a0);
});

When('I try to tag step 2 at timestamp {string}', function (a0) {
  try {
    tagStepTimestamp(this.lastRecipe, 2, a0);
    this.lastError = null;
  } catch (e) {
    this.lastError = e;
  }
});

Then('the tag is rejected', function () {
  assert.ok(this.lastError);
});

