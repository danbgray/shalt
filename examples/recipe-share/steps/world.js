import { setWorldConstructor } from '@cucumber/cucumber';

function World() {
  this.currentUser = null;
  this.lastRecipe = null;
  this.lastError = null;
  this.lastSaved = true;
  this.lastPacket = null;
  this.lastFreshLink = null;
  this.lastShare = null;
  this.lastView = null;
  this.lastTagResult = null;
  this.lastClip = null;
  this.users = new Map();
}

setWorldConstructor(World);

import { Before } from '@cucumber/cucumber';
import { resetStore } from '../src/store.js';
Before(function () { resetStore(); });
