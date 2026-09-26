import { Given, When, Then } from '@cucumber/cucumber';
import assert from 'node:assert/strict';

import { addIngredient, createRecipe, isPublic, openShareUrl, publish, setVisibility } from '../src/recipes.js';
Given('the recipe is private', function () {
  setVisibility(this.lastRecipe, 'private');
});

When('I publish {string}', function (a0) {
  this.lastShare = publish(a0);
});

Then('the recipe is public', function () {
  assert.ok(isPublic(this.lastRecipe));
});

Then('I receive a share URL matching {string}', function (a0) {
  assert.ok(String(this.lastShare && (this.lastShare.path || this.lastShare.shareUrl || '')).includes(a0.replace(/^\/r\//, '')) || String(this.lastShare && this.lastShare.path) === a0);
});

Given('a public recipe {string} at {string}', function (a0, a1) {
  this.lastRecipe = createRecipe('maya@example.com', a0);
  addIngredient(this.lastRecipe, 'salt');
  publish(this.lastRecipe);
});

When('an anonymous viewer opens {string}', function (a0) {
  this.lastView = openShareUrl(a0, null);
});

Given('a private recipe {string} owned by {string}', function (a0, a1) {
  this.lastRecipe = createRecipe(a1, a0);
});

Then('they see a not-found response', function () {
  assert.ok(this.lastView && (this.lastView.status === 'not-found' || this.lastView.notFound));
});

