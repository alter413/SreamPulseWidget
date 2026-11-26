import {mkdir, writeFile} from 'node:fs/promises';
import {dirname, join} from 'node:path';

const packages = [
  ['https://reactbits.dev/r/SquishSwitch-JS-CSS.json', 'SquishSwitch'],
  ['https://reactbits.dev/r/RubberSegment-JS-CSS.json', 'RubberSegment'],
  ['https://reactbits.dev/r/WakeSlider-JS-CSS.json', 'WakeSlider'],
  ['https://motion-primitives.com/c/text-effect.json', 'text-effect'],
];
for (const [url, name] of packages) {
  const response = await fetch(url);
  if (!response.ok) throw new Error('Registry ' + response.status + ': ' + url);
  const entry = await response.json();
  for (const file of entry.files) {
    const base = file.path.split('/').at(-1);
    if (!base.startsWith(name) || !/^[a-zA-Z0-9.-]+$/.test(base)) throw new Error('Unexpected registry file');
    const path = join('src', 'components', name === 'text-effect' ? 'core' : '', base);
    await mkdir(dirname(path), {recursive:true});
    await writeFile(path, file.content);
  }
}
