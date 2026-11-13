/** "Which paper said this" is a different question from "which paper is
 * this", and the Library panel on Home already answers it with the trigram
 * index. So the Projects search hands its query over instead of growing a
 * second full-text search: set the library query, ask Home to open its
 * Library panel, and switch screens.
 *
 * The order matters. The panel request has to be queued before the setting
 * flips, because flipping it is what mounts HomeScreen, and HomeScreen reads
 * the request as it mounts. */
import { libraryStore } from '../citations/libraryStore';
import { showHomePanel } from '../home/HomeScreen';
import { updateSettings } from '../../stores/settings';

export function searchFullText(query: string): void {
  libraryStore.setQuery(query);
  showHomePanel('library');
  updateSettings({ launchScreen: 'home' });
}
