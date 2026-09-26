/** Application module barrel — re-exports domain APIs matching contract/interface.md. */

export {
  createUser,
  createRecipe,
  tryCreateRecipe,
  getRecipe,
  addIngredient,
  setRecipeIngredients,
  addStep,
  changeStep,
  getStep,
  ingredientCount,
  stepCount,
  stepsInOrder,
  ingredientsOf,
  setVisibility,
  isPrivate,
  isPublic,
  publish,
  shareUrlFor,
  openShareUrl,
  resolveRecipe,
  ensureOwnedRecipe,
} from './recipes.js';

export {
  createPacket,
  getPacket,
  packetItemCount,
  packetRecipeTitle,
  setAssociateTag,
  clearAssociateTag,
  amazonFreshOrderLink,
  ensureRecipeWithIngredients,
} from './packets.js';

export {
  attachFullVideo,
  fullVideoUrl,
  tagStepTimestamp,
  stepTimestamp,
  openStepClip,
  parseTimestamp,
} from './video.js';

export {
  enablePatronage,
  patronageOffer,
  subscribePatron,
  cancelPatronage,
  isActivePatron,
  activePatronCount,
  setPatronOnly,
  viewRecipe,
} from './patronage.js';

export { resetStore, db, slugify, userEmail, ensureUser } from './store.js';
