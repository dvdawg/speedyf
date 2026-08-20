import { describe, expect, it } from 'vitest';
import { isTrustedLink, parseExternalLink, withTrustedHost } from './externalLink';

describe('parseExternalLink', () => {
  it('reads web links and lowercases the host', () => {
    expect(parseExternalLink('https://ArXiv.org/abs/1706.03762')).toEqual({
      href: 'https://arxiv.org/abs/1706.03762',
      host: 'arxiv.org',
    });
  });

  it('accepts mail links but gives them no host to remember', () => {
    const link = parseExternalLink('mailto:someone@example.edu');
    expect(link?.host).toBeNull();
  });

  it('refuses schemes that would reach the local machine', () => {
    for (const hostile of [
      'file:///etc/passwd',
      'javascript:alert(1)',
      'data:text/html,<script>alert(1)</script>',
      'vscode://file/Users/me/.ssh/id_rsa',
    ]) {
      expect(parseExternalLink(hostile), hostile).toBeNull();
    }
  });

  it('refuses malformed and hostless links', () => {
    expect(parseExternalLink('')).toBeNull();
    expect(parseExternalLink('not a url')).toBeNull();
    expect(parseExternalLink('https://')).toBeNull();
  });
});

describe('isTrustedLink', () => {
  const trusted = ['arxiv.org', 'example.edu'];

  it('matches a host the user has already agreed to', () => {
    expect(isTrustedLink({ href: 'https://arxiv.org/x', host: 'arxiv.org' }, trusted)).toBe(true);
  });

  it('does not let a lookalike host inherit that trust', () => {
    // The failure this guards against: a link that reads as arxiv.org at a
    // glance but resolves somewhere else entirely.
    for (const host of ['arxiv.org.example.com', 'notarxiv.org', 'evil-arxiv.org']) {
      expect(isTrustedLink({ href: `https://${host}/x`, host }, trusted), host).toBe(false);
    }
  });

  it('never treats a hostless link as trusted', () => {
    expect(isTrustedLink({ href: 'mailto:a@b.c', host: null }, trusted)).toBe(false);
  });
});

describe('withTrustedHost', () => {
  it('appends a new host and leaves an existing one alone', () => {
    expect(withTrustedHost(['a.com'], 'b.com')).toEqual(['a.com', 'b.com']);
    expect(withTrustedHost(['a.com'], 'a.com')).toEqual(['a.com']);
  });
});
