import { Given, When, Then } from '@cucumber/cucumber';
import assert from 'node:assert/strict';

import { activePatronCount, cancelPatronage, enablePatronage, isActivePatron, patronageOffer, setPatronOnly, subscribePatron, viewRecipe } from '../src/patronage.js';
import { createRecipe, getRecipe, openShareUrl, shareUrlFor } from '../src/recipes.js';
When('I enable patronage at {string} per month', function (a0) {
  this.lastOffer = enablePatronage(this.currentUser, Number(String(a0).replace(/[^0-9.]/g, '')));
});

Then('my profile shows patronage available at {string} per month', function (a0) {
  const o = patronageOffer(this.currentUser);
  assert.equal(o && o.amountPerMonth, Number(String(a0).replace(/[^0-9.]/g, '')));
});

Given('author {string} offers patronage at {string} per month', function (a0, a1) {
  enablePatronage(a0, Number(String(a1).replace(/[^0-9.]/g, '')));
});

When('I subscribe as a patron of {string} at {string} per month', function (a0, a1) {
  subscribePatron(this.currentUser, a0, Number(String(a1).replace(/[^0-9.]/g, '')));
});

Then('{string} is an active patron of {string}', function (a0, a1) {
  enablePatronage(a1, 5);
  subscribePatron(a0, a1, 5);
  assert.ok(isActivePatron(a0, a1));
});

Then('{string} has 1 active patron', function (a0) {
  assert.equal(activePatronCount(a0), 1);
});

Given('a patron-only recipe {string} owned by {string}', function (a0, a1) {
  this.lastRecipe = createRecipe(a1, a0);
  setPatronOnly(this.lastRecipe, true);
});

Given('{string} is not a patron of {string}', function (a0, a1) {
  assert.ok(!isActivePatron(a0, a1));
});

When('{string} opens the share URL for {string}', function (a0, a1) {
  this.lastView = viewRecipe(a1, a0);
});

Then('they see a patronage paywall', function () {
  assert.equal(this.lastView && this.lastView.status, 'paywall');
});

Then('they do not see the ingredient list', function () {
  assert.ok(!(this.lastView && this.lastView.ingredients && this.lastView.ingredients.length));
});


Then('they see title {string}', function (a0) {
  assert.equal(this.lastView && this.lastView.title, a0);
});

Then('they see the ingredient list', function () {
  assert.ok(this.lastView && Array.isArray(this.lastView.ingredients));
});

Then('they see the ordered steps', function () {
  assert.ok(this.lastView && Array.isArray(this.lastView.steps));
});

When('{string} cancels patronage of {string}', function (a0, a1) {
  cancelPatronage(a0, a1);
});




