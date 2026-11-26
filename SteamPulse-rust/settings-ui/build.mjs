import {build} from 'esbuild';
import {readFile, writeFile} from 'node:fs/promises';

const bundling = await build({
  entryPoints: ['src/main.jsx'], bundle:true, minify:true, write:false, outdir:'dist', target:'chrome120', jsx:'automatic',
  tsconfig:'jsconfig.json', loader:{'.woff2':'dataurl'}, legalComments:'inline',
  define:{'process.env.NODE_ENV':'"production"'}
});
const css = bundling.outputFiles.find(f => f.path.endsWith('.css')).text;
const js = bundling.outputFiles.find(f => f.path.endsWith('.js')).text.replaceAll('</script', '<\\/script');
const html = await readFile('index.html','utf8');
await writeFile('../assets/settings.html', html.replace('/*CSS*/',()=>css).replace('/*JS*/',()=>js));