/** Which desktop the app is running on, for the few things that differ: the
 * menu bar (macOS only), the title bar's document state, and key names. */
export const IS_MAC = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);
