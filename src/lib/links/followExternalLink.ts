/** The one path from "user clicked a link in a PDF" to "a browser opened".
 *
 * Everything a document points at goes through here so the confirmation is
 * impossible to route around, and so a remembered host is remembered in one
 * place rather than at each call site. */
import { askExternalLink, showError } from '../../stores/modalStore';
import { settings, updateSettings } from '../../stores/settings';
import { engine } from '../transport/engine';
import { isTrustedLink, parseExternalLink, withTrustedHost } from './externalLink';

export async function followExternalLink(raw: string): Promise<void> {
  const link = parseExternalLink(raw);
  if (!link) {
    showError(`SpeedyF only opens web and mail links. This document points at: ${raw}`);
    return;
  }
  if (!isTrustedLink(link, settings.trustedLinkHosts)) {
    const choice = await askExternalLink(link);
    if (!choice.open) return;
    if (choice.remember && link.host) {
      updateSettings({ trustedLinkHosts: withTrustedHost(settings.trustedLinkHosts, link.host) });
    }
  }
  try {
    await engine.openExternalUrl(link.href);
  } catch {
    showError('Could not hand that link to your browser.');
  }
}
