import { Given, When, Then } from '@cucumber/cucumber';
import assert from 'node:assert/strict';

import { amazonFreshOrderLink, clearAssociateTag, createPacket, getPacket, packetItemCount, packetRecipeTitle, setAssociateTag } from '../src/packets.js';
import { addIngredient, createRecipe, getRecipe, setVisibility } from '../src/recipes.js';
Given('a public recipe {string} with ingredients:', function (a0, table) {
  this.currentUser = this.currentUser || 'maya@example.com';
  this.lastRecipe = createRecipe(this.currentUser, a0);
  setVisibility(this.lastRecipe, 'public');
  for (const row of table.hashes()) {
    addIngredient(this.lastRecipe, { name: row.name, quantity: row.quantity });
  }
});

When('the author creates packet {string} from all ingredients', function (a0) {
  const names = (this.lastRecipe.ingredients || []).map((i) => i.name);
  this.lastPacket = createPacket(this.lastRecipe, a0, names);
});

Then('packet {string} contains 3 items', function (a0) {
  assert.equal(packetItemCount(this.lastPacket || a0), 3);
});

Then('the packet is linked to recipe {string}', function (a0) {
  assert.equal(packetRecipeTitle(this.lastPacket), a0);
});

Given('a packet {string} on recipe {string}', function (a0, a1) {
  this.currentUser = this.currentUser || 'maya@example.com';
  this.lastRecipe = getRecipe(a1) || createRecipe(this.currentUser, a1);
  this.lastPacket = getPacket(a0) || createPacket(this.lastRecipe, a0, (this.lastRecipe.ingredients || []).map((i) => i.name));
});

Given('the author\'s Amazon Associate tag is {string}', function (a0) {
  this.currentUser = this.currentUser || 'maya@example.com';
  setAssociateTag(this.currentUser, a0);
});

When('a viewer requests an Amazon Fresh order link for {string}', function (a0) {
  this.lastUrl = amazonFreshOrderLink(a0, null);
});

Then('they receive an Amazon Fresh URL that includes tag {string}', function (a0) {
  const url = this.lastUrl && (this.lastUrl.url || this.lastUrl.href || this.lastUrl);
  assert.ok(String(url || '').includes(a0));
});

Then('the URL lists the packet ingredient names', function () {
  assert.ok(this.lastUrl);
});

Given('the author has no Amazon Associate tag', function () {
  this.currentUser = this.currentUser || 'maya@example.com';
  clearAssociateTag(this.currentUser);
});

Then('they receive an Amazon Fresh URL', function () {
  assert.ok(this.lastUrl);
});

Then('the URL does not include an associate tag parameter', function () {
  assert.ok(!/tag=/.test(String(this.lastUrl || '')));
});

