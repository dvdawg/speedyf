// @vitest-environment happy-dom
/** The toggle is the setting: choosing a side writes `launchScreen`, which
 * is what App reads to pick the screen. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', () => ({ engine: {} }));

const { default: LaunchToggle } = await import('./LaunchToggle');
const { settings, updateSettings } = await import('../../stores/settings');

afterEach(cleanup);

describe('LaunchToggle', () => {
  it('writes launchScreen and reflects it', () => {
    updateSettings({ launchScreen: 'home' });
    render(() => <LaunchToggle />);
    const home = screen.getByRole('radio', { name: 'Home' });
    const projects = screen.getByRole('radio', { name: 'Projects' });
    expect(home.getAttribute('aria-checked')).toBe('true');

    fireEvent.click(projects);
    expect(settings.launchScreen).toBe('projects');
    expect(projects.getAttribute('aria-checked')).toBe('true');
    expect(home.getAttribute('aria-checked')).toBe('false');

    fireEvent.click(home);
    expect(settings.launchScreen).toBe('home');
  });
});
