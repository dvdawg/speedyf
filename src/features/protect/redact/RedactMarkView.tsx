/** One redaction mark on the page: an outlined, hatched box per covered line
 * (or its area), shown filled - as it will print - once applying is armed. */
import { For } from 'solid-js';
import type { Annotation } from '../../../types/model';
import { inkOn, markBoxes } from './redactMarks';
import './redact.css';

export default function RedactMarkView(props: {
  annot: Annotation;
  zoom: number;
  pageHeight: number;
  selected: boolean;
  armed: boolean;
  interactive: boolean;
  onPointerDown: (e: PointerEvent) => void;
}) {
  return (
    <For each={markBoxes(props.annot)}>
      {(box) => (
        <div
          class="redact-mark"
          classList={{ 'is-selected': props.selected, 'is-armed': props.armed }}
          data-annot-id={props.annot.id}
          style={{
            left: `${box.x * props.zoom}px`,
            top: `${(props.pageHeight - box.y - box.h) * props.zoom}px`,
            width: `${box.w * props.zoom}px`,
            height: `${box.h * props.zoom}px`,
            '--redact-fill': props.annot.color,
            '--redact-ink': inkOn(props.annot.color),
            'pointer-events': props.interactive ? 'auto' : 'none',
          }}
          title={props.armed ? 'Redaction, applied when you save' : 'Redaction mark'}
          onPointerDown={(e) => props.onPointerDown(e)}
        >
          {props.armed && props.annot.text ? (
            <span class="redact-mark-text">{props.annot.text}</span>
          ) : null}
        </div>
      )}
    </For>
  );
}
