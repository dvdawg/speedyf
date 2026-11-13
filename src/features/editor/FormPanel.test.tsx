// @vitest-environment happy-dom
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, expect, it, vi } from 'vitest';
import { TabContext } from '../../app/TabContext';
import { createTab } from '../../stores/tabsStore';
import FormPanel from './FormPanel';

vi.mock('../forms/formStore', () => ({
  formFields: async () => [
    { name: 'answer', page: 0, kind: 'text', value: 'Original answer', readOnly: false },
  ],
}));
afterEach(cleanup);

it('includes in-progress field text in Save without requiring blur, with one undo step', async () => {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 1,
    path: '/tmp/form.pdf',
    name: 'form.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  const view = render(() => (
    <TabContext.Provider value={tab}>
      <FormPanel />
    </TabContext.Provider>
  ));
  const field = await view.findByRole('textbox');
  fireEvent.focus(field);
  fireEvent.input(field, { target: { value: 'Updated' } });
  fireEvent.input(field, { target: { value: 'Updated form answer' } });
  expect(tab.documentStore.buildEditPlan().form).toEqual([['answer', 'Updated form answer']]);
  expect(tab.documentStore.state.dirty).toBe(true);
  tab.documentStore.undo();
  expect((field as HTMLInputElement).value).toBe('Original answer');
  expect(tab.documentStore.state.dirty).toBe(false);
  tab.documentStore.redo();
  expect(tab.documentStore.buildEditPlan().form).toEqual([['answer', 'Updated form answer']]);
});
