import React, {memo, useEffect, useRef, useState, useDeferredValue} from 'react';
import {createRoot} from 'react-dom/client';
import {AnimatePresence, MotionConfig, motion, useReducedMotion} from 'motion/react';
import {TextEffect} from '@/components/core/text-effect';
import SquishSwitch from '@/components/SquishSwitch';
import RubberSegment from '@/components/RubberSegment';
import WakeSlider from '@/components/WakeSlider';
import messages from '../../SteamPulse/Locales.json';
import ChoiceMenu from './ChoiceMenu';
import ColorMenu from './ColorMenu';
import './styles.css';

const web = window.chrome?.webview;
const send = (action, fields = {}) => web?.postMessage({action,...fields});
const flags = ['Downloads','Games'];
const details = ['Icon','Name','Speed','RemainingTime','Percent','Bar','Network','Bits'];
const tabs = ['display','appearance','position','games','friendsTab','notifications','general'];
const languages = {
  en:'English', zh:'简体中文', es:'Español', hi:'हिन्दी', ar:'العربية', fr:'Français',
  pt:'Português', bn:'বাংলা', ru:'Русский', ja:'日本語', de:'Deutsch', ko:'한국어', id:'Bahasa Indonesia',
};
const options = {
  GlassDepth:[-5,5,.1], GlassBlur:[0,10,.1,' px'], GlassDispersion:[0,3,.1], Shine:[0,1,.01,'%'],
  SettingsGlassDepth:[-5,5,.1], SettingsGlassBlur:[0,10,.1,' px'], SettingsGlassDispersion:[0,3,.1],
  SettingsShine:[0,1,.01,'%'], SettingsGlassLight:[0,360,1,'°'],
  GlassLight:[0,360,1,'°'], GlassTint:[0,1,.01,'%'], SettingsTint:[.05,1,.01,'%'], Opacity:[.1,1,.01,'%'],
  Width:[200,550,1,' px'], Radius:[0,11,1,' px'], Angle:[0,360,1,'°'], HorizontalPosition:[0,100,.5,'%'], MinDiscount:[5,95,1,'%'], MetricsInterval:[1,5,1,'s'], FriendCycle:[3,20,1,'s'],
};
const palette = {
  light:{track:'#dae0e8', on:'#2153a5', knob:'#fff', stroke:'#b4c4d9', fill:'#2153a5',
    tabs:'#e6e9ee', thumb:'#202a3a', ink:'#222b38', active:'#fff'},
  dark:{track:'#444951', on:'#247af1', knob:'#fff', stroke:'#373e49', fill:'#78aeff',
    tabs:'#252930', thumb:'#e9effa', ink:'#edf3ff', active:'#172033'},
};
const defaultD = {name:'Portal 2',percent:64.2,speed:14811341,remaining:1080};
const tone = (p,k) => p.LightTheme ? (
  {Background:'#101929',BackgroundEnd:'#1F3851',Accent:'#54DAFE',AccentEnd:'#81A5FF',Text:'#F1F7FF'}[k] === p[k] ?
    {Background:'#f7f9fd',BackgroundEnd:'#d9e7fb',Accent:'#2453a6',AccentEnd:'#6049a0',Text:'#152033'}[k] : p[k]
) : p[k];

const Heading = memo(function Heading({children, as, locale}) {
  const reduce = useReducedMotion();
  const Tag = as || 'h2';
  return reduce ? <Tag>{children}</Tag> :
    <TextEffect as={Tag} per={["ar","hi","bn"].includes(locale)?"word":"char"} preset="fade" speedReveal={1.45} speedSegment={1.35}>{children}</TextEffect>;
});

function Remaining({seconds,t}) {
  const minutes = Math.ceil(seconds / 60);
  if(minutes >= 1440) return <>≈{Math.floor(minutes/1440)} {t('days')} {Math.floor(minutes/60)%24} {t('hours')}</>;
  if(minutes >= 60) return <>≈{Math.floor(minutes/60)} {t('hours')} {minutes%60} {t('minutes')}</>;
  return seconds >= 60 ? <>≈{minutes} {t('minutes')}</> : <>≈{Math.ceil(seconds)} {t('seconds')}</>;
}

function Preview({p,t,locale}) {
  const [status,setStatus] = useState(null);
  useEffect(()=>{
    const incoming = e => { if(e.data?.type==='status') setStatus(e.data); };
    web?.addEventListener('message',incoming);
    send('status');
    return ()=>web?.removeEventListener('message',incoming);
  },[]);
  const [mode,setMode] = useState('download');
  const isIgnored = id => p.IgnoredGames?.includes(id);
  const sample = status?.download?.name ? {...defaultD,name:status.download.name} : defaultD;
  const d = mode === 'download' && p.Downloads ? sample : mode === 'status' && p.Downloads && !isIgnored(status?.download?.id) ? status?.download : null;
  const game = mode === 'inGame' && p.Games ? (status?.game || {name:'Portal 2'}) :
    mode === 'status' && p.Games && !isIgnored(status?.game?.id) ? status?.game : null;
  const sale = mode === 'sale' && p.Discounts && p.DiscountCard;
  const activity = p.Widget && ((d && p.Downloads) || game || sale || mode === 'status');
  const num = new Intl.NumberFormat(locale,{maximumFractionDigits:1});
  const style = {
    color:tone(p,'Text'), '--pulse-accent':tone(p,'Accent'), borderRadius:p.Radius, borderColor:p.WidgetBorder&&!p.AirMode?"#ffffff52":"transparent",
    maxWidth: p.Width,
    background: p.AirMode ? "none" : p.Glass ? undefined : p.Gradient
      ? 'linear-gradient('+p.Angle+'deg,'+tone(p,'Background')+','+tone(p,'BackgroundEnd')+')' : tone(p,'Background'),
  };
  return <section className="preview" aria-label={t('preview')}>
    <div className="preview-head"><span>{t('preview')}</span>
      <ChoiceMenu label={t('preview')} value={mode} onChange={setMode}
        options={['download','inGame','sale','status'].map(m=>[m,t(m)])}/>
    </div>
    <div className={'preview-deck'+(p.Glass&&!p.AirMode?' preview-deck--glass':'')}>
      {!activity ? <div className="muted">{t('inactivity')}</div> :
        <div className={'preview-widget'+(p.Glass&&!p.AirMode?' preview-widget--glass':'')} style={style}>
          {d && p.Downloads && <div className="preview-line">
            {p.Icon && <span className="preview-icon" aria-hidden="true">↓</span>}
            <div className="preview-data">
              <div className="preview-fields">
                <b>{p.Name ? d.name : t(d.installing?'installing':'download')}</b>
                <span>
                  {p.Percent && d.percent != null && <span>{d.estimated?'≈':''}{num.format(d.percent)}%</span>}
                  {p.Speed && d.speed && <span>{num.format(p.Bits?d.speed*8/1000000:d.speed/1048576)} {p.Bits?'Mbps':'MiB/s'}</span>}
                  {p.RemainingTime && d.remaining && <span><Remaining seconds={d.remaining} t={t}/></span>}
                </span>
              </div>
              {p.Bar && <div className="preview-progress"><div style={{width:(d.percent||0)+'%'}}/></div>}
            </div>
          </div>}
          {game && <div className="preview-line">
            {p.Icon && <span className="preview-icon" aria-hidden="true">▶</span>}
            <b>{p.Name ? game.name : t('inGame')}</b>
            {p.MonitorSystem && <span className="preview-metrics">
              {p.MonitorCpu&&<span>CPU {status?.load?.cpu??24}%</span>}
              {p.MonitorRam&&<span>RAM {p.RamGiB ? ((status?.load?.memoryGiB??9).toFixed(1)+' GiB') : ((status?.load?.memoryPercent??43)+'%')}</span>}
            </span>}
          </div>}
          {sale && <div className="preview-line">{p.Icon && <span className="preview-icon">%</span>}<b>{p.Name?'Hades':t('sale')}</b><span>−60%</span></div>}
          {!d && !game && !sale && <b>{t(status?.active?'online':status?.available?'closed':'notFound')}</b>}
        </div>}
    </div>
  </section>;
}

function GlassImage() {
  const ref = useRef(null);
  useEffect(()=>{
    const image = ref.current;
    let loading = false;
    let latest = null;
    function request() {
      if(loading || !latest || image.getAttribute('src')===latest) return;
      loading = true;
      image.src = latest;
    }
    function done(e) {
      loading = false;
      if(e.type==='load' && latest) image.hidden=false;
      request();
    }
    function incoming(e) {
      if(e.data?.type!=='glass') return;
      latest = e.data.frame;
      if(!latest) {
        loading = false;
        image.hidden = true;
        image.removeAttribute('src');
      } else request();
    }
    image.addEventListener('load',done);
    image.addEventListener('error',done);
    web?.addEventListener('message',incoming);
    send('glassReady');
    return ()=>{
      image.removeEventListener('load',done);
      image.removeEventListener('error',done);
      web?.removeEventListener('message',incoming);
    };
  },[]);
  return <img ref={ref} className="live-backdrop" alt="" hidden />;
}

function FriendsList({t}) {
  const [live,setLive]=useState(null);
  const [page,setPage]=useState(0);
  const size=12;
  useEffect(()=>{
    const receive=e=>{if(e.data?.type==='status')setLive(e.data);};
    web?.addEventListener('message',receive);
    send('status');
    return ()=>web?.removeEventListener('message',receive);
  },[]);
  if(!live) return <p className="muted">{t('friendsLoading')}</p>;
  if(!live.active || live.friends?.available===false) return <p className="muted">{t('friendsUnavailable')}</p>;
  if(!live.friends) return <p className="muted">{t('friendsLoading')}</p>;
  const playing=live.friends.playing;
  if(!playing.length) return <p className="muted">{t('noFriendGames')}</p>;
  const max=Math.ceil(playing.length/size);
  const index=Math.min(page,max-1);
  return <>
    <ol className="library">{playing.slice(index*size,(index+1)*size).map((row,i)=>
      <li className="row friend-entry" key={index*size+i}><strong title={row.name}>{row.name}</strong><span title={row.game}>{row.game}</span></li>)}</ol>
    {max>1&&<div className="friend-pagination">
      <button aria-label={t('previousPage')} disabled={index===0} onClick={()=>setPage(index-1)}>‹</button>
      <span>{index+1} / {max}</span>
      <button aria-label={t('nextPage')} disabled={index===max-1} onClick={()=>setPage(index+1)}>›</button>
    </div>}
  </>;
}

function LocalLibraryRow({game,t,colors}) {
  return <li className="row local-game">
    <div>
      <input type="text" aria-label={t('localGameName')} defaultValue={game.name} maxLength={90}
        onKeyDown={e=>{if(e.key==='Enter')e.currentTarget.blur();}}
        onBlur={e=>{const value=e.target.value.trim();if(value&&value!==game.name)send('renameLocalGame',{id:game.id,name:value});
          else e.target.value=game.name;}} />
      <small title={game.file}>{t('localGames')} · {game.file}</small>
    </div>
    <div className="local-actions">
      <button onClick={()=>send('removeLocalGame',{id:game.id})}>{t('removeLocalGame')}</button>
      <SquishSwitch checked={!!game.ignored} onChange={ignored=>send('ignoreLocalGame',{id:game.id,ignored})} ariaLabel={game.name}
        width={46} height={24} radius={12} trackColor={colors.track} trackOnColor={colors.on} thumbOnColor={colors.knob}/>
    </div>
  </li>;
}

function App() {
  const [model,setModel] = useState(null);
  const [section,setSection] = useState('display');
  const [displays,setDisplays] = useState([]);
  const [games,setGames] = useState(null);
  const [localGames,setLocalGames] = useState([]);
  const [libraryFailed,setLibraryFailed] = useState(false);
  const [idleError,setIdleError] = useState('');
  const [glassUnavailable,setGlassUnavailable] = useState(false);
  const [query,setQuery] = useState('');
  const [confirm,setConfirm] = useState(false);
  const body = useRef(null);
  const pending = useRef(new Map());
  const queuedFrame = useRef(0);
  const reduce = useReducedMotion();
  const locale = model?.language || 'en';
  const p = model?.entries || {};
  const colors = p.LightTheme ? palette.light : palette.dark;
  const t = key => messages[locale]?.[key] || messages.en[key] || key;
  const deferredQuery = useDeferredValue(query);
  const [countryDraft,setCountryDraft] = useState(null);

  useEffect(()=>{
    function incoming(e) {
      const d=e.data;
      if(d?.type==='settings') setModel(d);
      else if(d?.type==='localGames') setLocalGames(d.games);
      else if(d?.type==='monitors') setDisplays(d.displays);
      else if(d?.type==='library') {setGames(d.games);setLibraryFailed(false);}
      else if(d?.type==='libraryError') {setLibraryFailed(true);setGames([]);}
      else if(d?.type==='error') setIdleError(d.key);
      else if(d?.type==='save') setModel(m=>m&&({...m,saveFailed:d.failed}));
      else if(d?.type==='glass') setGlassUnavailable(!d.frame);
    }
    web?.addEventListener('message',incoming);
    send('ready');
    return ()=>{web?.removeEventListener('message',incoming); cancelAnimationFrame(queuedFrame.current);};
  },[]);
  useEffect(()=>{
    document.documentElement.lang = locale;
    document.documentElement.dir = locale === 'ar' ? 'rtl' : 'ltr';
    document.documentElement.dataset.theme = p.LightTheme ? 'light' : 'dark';
  },[locale,p.LightTheme]);
  useEffect(()=>{
    body.current?.scrollTo(0,0);
    if(section==='games') send('library');
  },[section]);

  function change(key,value,coalesce=false) {
    setIdleError('');
    setModel(m=>({...m,entries:{...m.entries,[key]:value}}));
    if(!coalesce) {pending.current.delete(key);send('change',{key,value});return;}
    pending.current.set(key,value);
    if(queuedFrame.current) return;
    queuedFrame.current=requestAnimationFrame(()=>{
      queuedFrame.current=0;
      for(const [entry,v] of pending.current) send('change',{key:entry,value:v});
      pending.current.clear();
    });
  }
  function ignore(id,ignored) {
    setModel(m=>({...m,entries:{...m.entries,IgnoredGames:ignored?[...m.entries.IgnoredGames,id]:m.entries.IgnoredGames.filter(i=>i!==id)}}));
    send('ignore',{id,ignored});
  }
  function switchInput(key, override) {
    return <SquishSwitch checked={!!p[key]} onChange={v=>change(key,v)} ariaLabel={t(key)}
      width={46} height={24} radius={12} speed={62} trackColor={colors.track} trackOnColor={colors.on} thumbOnColor={colors.knob} {...override}/>;
  }
  function toggle(key) {return <div key={key} className="row"><span>{t(key)}</span>{switchInput(key)}</div>;}
  function slide(key, labelKey = key) {
    const [min,max,step,suffix=''] = options[key];
    const val = p[key] ?? (key === 'HorizontalPosition' ? 83 : min);
    const format = v => new Intl.NumberFormat(locale,{maximumFractionDigits:1}).format(suffix==='%'&&max===1?v*100:v)+(suffix==='s'?' '+t('seconds'):suffix);
    return <div key={key} className="row"><span>{t(labelKey)}</span>
      <div className="slider-wrap" dir="ltr">
        <WakeSlider ariaLabel={t(labelKey)} value={val} min={min} max={max} step={step} onChange={v=>change(key,v,true)}
          bars={24} height={33} restHeight={9} gap={3} fillColor={colors.fill} trackColor={colors.stroke} />
        <output>{format(val)}</output>
      </div>
    </div>;
  }
  function choose(key,items,selection=p[key]??'') {
    const label = key==='MonitorName' ? t('Monitor') : t(key);
    return <div className="row"><label htmlFor={key}>{label}</label>
      <ChoiceMenu id={key} label={label} value={selection} options={items} onChange={value=>change(key,value)}/>
    </div>;
  }
  function swatch(key) {return <div className="row" key={key}><span>{t(key)}</span>
    <ColorMenu label={t(key)} value={tone(p,key)||'#161923'} onChange={value=>change(key,value)} done={t('done')} colors={colors}/>
  </div>;}

  if(!model) return <div className="boot"><strong>Steam Pulse</strong><progress aria-label="Loading settings"/></div>;
  const filter=deferredQuery.toLocaleLowerCase().trim();
  const visibleGames=[
    ...(games||[]).map(game=>({...game, source:'steam'})),
    ...localGames.map(game=>({...game, source:'local'}))
  ].filter(game=>game.name.toLocaleLowerCase().includes(filter) ||
    (game.source==='steam'?String(game.id).includes(filter):game.file.toLocaleLowerCase().includes(filter)))
    .sort((a,b)=>a.name.localeCompare(b.name, locale));
  const themeLabel=t(p.LightTheme?'toDark':'toLight');
  const monitorOptions = [['',t('primary')], ...displays.map((display,n)=>[
    display.name, (display.primary?t('primary'):t('Monitor')+' '+(n+1))+' · '+display.width+'×'+display.height
  ])];
  const chosenMonitor = displays.some(m=>m.name===p.MonitorName) ? p.MonitorName : '';
  return <div className={'shell'+(p.SettingsGlass?' shell--glass':'')} style={{'--glass-tint':p.SettingsTint}}>
    <GlassImage/>
    <div className="surface" aria-hidden="true"/>
    <header>
      <div className="mark" aria-hidden="true"><svg viewBox="0 0 32 32" fill="none"><path d="M4 18h6l3-8 5 13 3-7h7" stroke="currentColor" strokeWidth="2.3" strokeLinejoin="round" strokeLinecap="round"/></svg></div>
      <Heading as="h1">Steam Pulse</Heading>
    </header>
    <Preview p={p} t={t} locale={locale}/>
    <nav aria-label={t('settings')} dir="ltr">
      <RubberSegment className="sections" value={section} onChange={setSection}
        items={tabs.map(key=>({value:key,label:<span dir={locale==='ar'?'rtl':'auto'}>{t(key)}</span>}))}
        aria-label={t('settings')} size="lg" radius={12} trackColor={colors.tabs} thumbColor={colors.thumb}
        textColor={colors.ink} activeTextColor={colors.active} speed={1.18}/>
    </nav>
    <main ref={body}>
      <div key={section} className="page">
        {section==='display' && <>
          <Heading locale={locale}>{t('status')}</Heading>{flags.map(toggle)}
          <Heading locale={locale}>{t('fields')}</Heading>{details.map(toggle)}
          <Heading locale={locale}>{t('MonitorSystem')}</Heading>{toggle('MonitorSystem')}
          {toggle('MonitorCpu')}{toggle('MonitorRam')}{toggle('RamGiB')}{slide('MetricsInterval')}
        </>}
        {section==='appearance' && <>
          <Heading locale={locale}>{t('glassGroup')}</Heading>
          {toggle('AirMode')}
          <div className="glass-profiles">
            <section className="glass-profile" aria-label={t('Glass')}>
              {toggle('Glass')}
              {['GlassDepth','GlassBlur','GlassDispersion','Shine','GlassLight','GlassTint'].map(key=>slide(key))}
            </section>
            <section className="glass-profile" aria-label={t('SettingsGlass')}>
              {toggle('SettingsGlass')}
              {p.SettingsGlass && glassUnavailable && <p className="notice">{t('glassUnavailable')}</p>}
              {[
                ['SettingsGlassDepth','GlassDepth'],['SettingsGlassBlur','GlassBlur'],
                ['SettingsGlassDispersion','GlassDispersion'],['SettingsShine','Shine'],
                ['SettingsGlassLight','GlassLight'],['SettingsTint','SettingsTint']
              ].map(([key,label])=>slide(key,label))}
            </section>
          </div>
          <Heading locale={locale}>{t('shape')}</Heading>{['Opacity','Width','Radius'].map(key=>slide(key))}{toggle('WidgetBorder')}{toggle('Gradient')}{slide('Angle')}
          {['Background','BackgroundEnd','Accent','AccentEnd','Text'].map(swatch)}
        </>}
        {section==='position' && <>
          <Heading locale={locale}>{t('position')}</Heading><p className="muted">{t('dragHelp')}</p>{slide('HorizontalPosition')}
          <div className="row"><button onClick={()=>change('HorizontalPosition',null)}>{t('autoPosition')}</button></div>
          {choose('MonitorName',monitorOptions,chosenMonitor)}{toggle('ClickThrough')}
        </>}
        {section==='games' && <>
          <div className="local-heading"><Heading locale={locale}>{t('games')}</Heading>
            <button onClick={()=>send('addLocalGame')}>{t('addLocalGame')}</button>
          </div>
          <p className="muted">{t('libraryHelp')}</p>
          <div className="library-controls">
            <input type="search" aria-label={t('searchGames')} placeholder={t('searchGames')} value={query} onChange={e=>setQuery(e.target.value)}/>
            <button onClick={()=>{setGames(null);setLibraryFailed(false);send('library',{refresh:true});}}>{t('refreshLibrary')}</button>
          </div>
          {libraryFailed&&<p role="status">{t('libraryError')}</p>}
          {!games&&!libraryFailed&&<p role="status">{t('loadingLibrary')}</p>}
          {!visibleGames.length&&games&&!libraryFailed&&<p role="status">{t('noGames')}</p>}
          <ol className="library">{visibleGames.map(game=>game.source==='local'
            ? <LocalLibraryRow key={'local-'+game.id} game={game} t={t} colors={colors}/>
            : <li className="row" key={'steam-'+game.id}>
                <div><strong title={game.name}>{game.name}</strong>{game.installed&&<small>{t('installed')}</small>}</div>
                <SquishSwitch checked={p.IgnoredGames.includes(game.id)} onChange={v=>ignore(game.id,v)} ariaLabel={game.name}
                  width={46} height={24} radius={12} trackColor={colors.track} trackOnColor={colors.on} thumbOnColor={colors.knob}/>
              </li>)}</ol>
        </>}
        {section==='friendsTab' && <>
          <Heading locale={locale}>{t('friendsTab')}</Heading>{toggle('Friends')}{slide('FriendCycle')}
          {p.Friends&&<FriendsList t={t}/>}
        </>}
        {section==='notifications' && <>
          <Heading locale={locale}>{t('notifications')}</Heading>{toggle('Discounts')}{toggle('DiscountCard')}{slide('MinDiscount')}
          <label className="row"><span>{t('Country')}</span><input className="country" maxLength={2} placeholder="US"
            value={countryDraft??p.Country} onChange={e=>{
              const code = e.target.value.toUpperCase().replace(/[^A-Z]/g,'');
              setCountryDraft(code);
              if(code.length!==1) change('Country',code);
            }} onBlur={()=>setCountryDraft(null)}/></label>
        </>}
        {section==='general' && <>
          <Heading locale={locale}>{t('general')}</Heading>
          {choose('Language',[['',t('system')],...Object.entries(languages)])}{toggle('AutoStart')}
          <p className="credits"><button onClick={()=>send('icons')}>UIcons by Flaticon</button></p>
        </>}
      </div>
    </main>
    <footer>
      <button onClick={()=>setConfirm(!confirm)}>{t('reset')}</button>
      <button className="theme-button" aria-label={themeLabel} title={themeLabel} onClick={()=>change('LightTheme',!p.LightTheme)}>
        <AnimatePresence initial={false} mode="wait">
          <motion.i key={String(p.LightTheme)} className={p.LightTheme?'fi fi-rc-moon-stars':'fi fi-br-brightness'} aria-hidden="true"
            initial={reduce?false:{opacity:0,scale:.62,rotate:-55}} animate={{opacity:1,scale:1,rotate:0}}
            exit={{opacity:0,scale:.66,rotate:48}} transition={{duration:reduce?0:.15}}/>
        </AnimatePresence>
      </button>
      <span className="save-label" role="status">{idleError?t(idleError):model.saveFailed?t('saveError'):t('saved')}</span>
      {model.version&&<span className="build-version" dir="ltr">v{model.version}</span>}
      <button className="primary" onClick={()=>send('close')}>{t('done')}</button>
    </footer>
    {confirm&&<div className="confirm" role="alertdialog" aria-modal="true" aria-label={t('resetPrompt')}>
      <p>{t('resetPrompt')}</p><button onClick={()=>setConfirm(false)}>{t('done')}</button>
      <button className="primary" onClick={()=>{setConfirm(false);send('reset');}}>{t('reset')}</button>
    </div>}
  </div>;
}
createRoot(document.getElementById('root')).render(<MotionConfig reducedMotion="user"><App/></MotionConfig>);
