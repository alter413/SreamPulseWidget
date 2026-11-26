import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const base = 'https://cdn-uicons.flaticon.com/4.0.0/';
const temp = await mkdtemp(join(tmpdir(), 'steampulse-uicons-'));
try {
  for (const [style, name, scalar] of [
    ['uicons-regular-chubby','moon','ED68'],['uicons-bold-rounded','sun','E2EF'],['uicons-solid-rounded','user','F4F9']
  ]) {
    const response = await fetch(base + style + '/webfonts/' + style + '.woff2');
    if (!response.ok) throw new Error('Flaticon ' + response.status);
    const input = join(temp, name + '.woff2');
    await writeFile(input, Buffer.from(await response.arrayBuffer()));
    const build = spawnSync('python', ['-m','fontTools.subset',input,'--unicodes=U+'+scalar,
      '--flavor=woff2','--output-file=src/'+name+'.woff2'],{stdio:'inherit'});
    if (build.error) throw build.error;
    if (build.status !== 0) throw new Error('fontTools failed. Install fonttools[woff].');
  }
} finally {
  await rm(temp,{recursive:true,force:true});
}
