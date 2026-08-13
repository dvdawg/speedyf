/** Placeholder surface. No extension system exists yet — this panel exists so
 * the concept has a home once one does. */
import { IconExtension } from '../../components/icons';

export default function ExtensionsPanel() {
  return (
    <>
      <div class="home-panel-head">
        <h2>Extensions</h2>
      </div>
      <div class="home-panel-body">
        <div class="home-placeholder">
          <span class="home-placeholder-icon" aria-hidden="true">
            <IconExtension />
          </span>
          <p class="home-placeholder-title">No extensions installed</p>
          <p>Extensions will let you add tools and viewers to SpeedyF.</p>
        </div>
      </div>
    </>
  );
}
