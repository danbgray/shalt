/** Patronage API lives in patronage.js per contract; re-export for safety. */
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
