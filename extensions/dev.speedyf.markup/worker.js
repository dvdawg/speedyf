// Markup - referee comments on someone else's paper.
//
// Everything Markup does is drawn and run by SpeedyF itself
// (src/features/markup): review mode, the comments on the page and in the
// drawer, their labels and numbering, and the export. This extension is the
// switch that turns it on or off from the Extensions panel; it contributes
// no commands of its own, so this worker is never started.
//
// It never touches the network.
//
// This file is ASCII on purpose: it carries no character outside it.

self.onmessage = () => {
  // Nothing to answer: SpeedyF never invokes a command here.
};
