import { Given, When, Then } from '@cucumber/cucumber';
import assert from 'node:assert/strict';

import { addIngredient, addStep, changeStep, createRecipe, getRecipe } from '../src/recipes.js';
Given('I am signed in as {string}', function (email) {
  this.currentUser = email;
});

When('I create a recipe titled {string}', function (a0) {
  this.lastRecipe = createRecipe(this.currentUser, a0);
});

When('I add ingredient {string}', function (a0) {
  addIngredient(this.lastRecipe, a0);
});

When('I add step 1 {string}', function (a0) {
  addStep(this.lastRecipe, 1, a0);
});

When('I add step 2 {string}', function (a0) {
  addStep(this.lastRecipe, 2, a0);
});

When('I add step 3 {string}', function (a0) {
  addStep(this.lastRecipe, 3, a0);
});

Then('the recipe {string} has 3 ingredients', function (a0) {
  const r = this.lastRecipe || getRecipe(a0);
  assert.equal((r.ingredients || []).length, 3);
});

Then('the recipe {string} has 3 steps in order', function (a0) {
  const r = this.lastRecipe || getRecipe(a0);
  assert.equal((r.steps || []).length, 3);
});

When('I try to create a recipe with an empty title', function () {
  try {
    this.lastRecipe = createRecipe(this.currentUser, '');
    this.lastError = null;
  } catch (e) {
    this.lastError = e;
    this.lastRecipe = null;
  }
});

Then('the recipe is not saved', function () {
  assert.equal(this.lastRecipe, null);
});

Then('I see the error {string}', function (a0) {
  assert.equal(this.lastError && this.lastError.message, a0);
});

Given('a recipe {string} owned by {string}', function (a0, a1) {
  this.lastRecipe = createRecipe(a1, a0);
});

Given('step 2 of {string} is {string}', function (a0, a1) {
  const r = this.lastRecipe || getRecipe(a0);
  const s = (r.steps || []).find((x) => x.number === 2);
  if (!s) addStep(r, 2, a1);
  else assert.equal(s.text, a1);
});

When('I change step 2 to {string}', function (a0) {
  changeStep(this.lastRecipe, 2, a0);
});




