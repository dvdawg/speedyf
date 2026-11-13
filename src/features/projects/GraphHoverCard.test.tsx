// @vitest-environment happy-dom
/** The hover card typesets TeX in titles and abstracts, and nothing in a
 * title is ever parsed as HTML. */
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import GraphHoverCard from './GraphHoverCard';
import { EVANS, paperFor } from './testFixtures';

afterEach(cleanup);

const card = (title: string, abstract: string | null) => {
  const paper = paperFor('g');
  paper.meta.abstract = abstract === null ? null : { value: abstract, source: 'crossref' };
  return render(() => (
    <GraphHoverCard
      id="card"
      row={{ ...EVANS, title }}
      label="ES02"
      paper={paper}
      cites={1}
      citedBy={0}
      left={0}
      top={0}
      width={300}
      side="right"
    />
  ));
};

describe('GraphHoverCard math', () => {
  it('typesets math in the title and the abstract', async () => {
    const { container } = card('Splitting H$_{2}$O', 'Rates grow as $N^2$ here.');
    await waitFor(() => {
      expect(container.querySelector('.graph-card-title .katex')).not.toBeNull();
      expect(container.querySelector('.graph-card-abstract .katex')).not.toBeNull();
    });
    expect(container.querySelector('.graph-card-title')!.textContent).toMatch(/^Splitting H.*O$/);
    expect(container.querySelector('.graph-card-abstract')!.textContent).toContain('here.');
  });

  it('shows markup in a title as text', () => {
    const { container } = card('<img src=x onerror="alert(1)"> costs $5 and $10', null);
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('.graph-card-title')!.textContent).toBe(
      '<img src=x onerror="alert(1)"> costs $5 and $10'
    );
  });
});
