import React, {useEffect,useId,useRef,useState} from 'react';
import {createPortal} from 'react-dom';
import {ShinyButton} from './MagicControls';
import {AnimatePresence,motion,useReducedMotion} from 'motion/react';
import {placePopup} from './ChoiceMenu';
import WakeSlider from '@/components/WakeSlider';

const presets = ['#FFFFFF','#DCE7F7','#98A9C6','#34415C','#171E2A','#0C1119',
  '#54DAFE','#4889F4','#8668E9','#E877AE','#F59A6B','#5BD5AC'];
const valid = hex => /^#[0-9a-f]{6}$/i.test(hex);

export default function ColorMenu({label,value,onChange,done,colors}) {
  const id=useId();
  const trigger=useRef(null);
  const panel=useRef(null);
  const [open,setOpen]=useState(false);
  const [position,setPosition]=useState(null);
  const [draft,setDraft]=useState(value);
  const reduced=useReducedMotion();

  function close(focus=false) {
    setOpen(false);
    if(focus) trigger.current?.focus();
  }
  function measure() {
    if(!trigger.current) return;
    const next=placePopup(trigger.current,262,252);
    if(next) setPosition(next);
    else close();
  }
  function toggle() {
    if(open) {close();return;}
    setDraft(value);
    measure();
    setOpen(true);
  }
  function apply(hex) {
    if(!valid(hex)) return;
    onChange(hex.toUpperCase());
    close(true);
  }

  useEffect(()=>{
    if(!open) return;
    panel.current?.querySelector("input")?.focus({preventScroll:true});
    function outside(e) {
      if(!trigger.current?.contains(e.target)&&!panel.current?.contains(e.target)) close();
    }
    const blur=()=>close();
    function keys(e) {
      if(e.key==='Escape') { e.preventDefault(); close(true); }
    }
    document.addEventListener('pointerdown',outside,true);
    document.addEventListener('keydown',keys);
    window.addEventListener('resize',measure);
    window.addEventListener('scroll',measure,true);
    window.addEventListener('blur',blur);
    return ()=>{
      document.removeEventListener('pointerdown',outside,true);
      document.removeEventListener('keydown',keys);
      window.removeEventListener('resize',measure);
      window.removeEventListener('scroll',measure,true);
      window.removeEventListener('blur',blur);
    };
  },[open]);

  const shown=valid(draft)?draft:value;
  const rgb=[1,3,5].map(index=>parseInt(shown.slice(index,index+2),16)||0);
  function updateChannel(index,value) {
    const channels=rgb.map((c,i)=>(i===index?value:c).toString(16).padStart(2,'0'));
    setDraft('#'+channels.join('').toUpperCase());
  }

  return <>
    <ShinyButton ref={trigger} type="button" className="choice-trigger color-trigger" aria-haspopup="dialog" aria-expanded={open}
      aria-controls={open?id:undefined} aria-label={label} onClick={toggle}>
      <span className="color-chip" style={{background:value}} aria-hidden="true"/>
      <code dir="ltr">{value}</code>
      <svg className="choice-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4 6 4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/></svg>
    </ShinyButton>
    {createPortal(<AnimatePresence>
      {open&&position&&<motion.div ref={panel} role="dialog" id={id} aria-label={label} className="choice-list color-panel"
        style={{left:position.left,width:position.width,top:position.top,bottom:position.bottom,maxHeight:position.maxHeight,
          transformOrigin:position.up?'bottom center':'top center'}}
        initial={reduced?false:{opacity:0,scale:.97,y:position.up?4:-4}}
        animate={{opacity:1,scale:1,y:0}}
        exit={reduced?{opacity:0}:{opacity:0,scale:.98,y:position.up?3:-3}}
        transition={{duration:reduced?0:.17,ease:[.2,.72,.35,1]}}>
        <div className="color-presets">
          {presets.map(hex=><button key={hex} className="color-preset" type="button" aria-label={hex} title={hex}
            style={{background:hex}} onClick={()=>apply(hex)}/>)}
        </div>
        <div className="color-channels" dir="ltr">
          {['R','G','B'].map((channel,i)=><div className="color-channel" key={channel}>
            <span>{channel}</span>
            <WakeSlider ariaLabel={channel} value={rgb[i]} min={0} max={255} step={1} onChange={v=>updateChannel(i,v)}
              bars={20} height={27} restHeight={8} gap={2} fillColor={['#E38388','#69B6A1','#749CE4'][i]} trackColor={colors.stroke}/>
            <output>{rgb[i]}</output>
          </div>)}
        </div>
        <form className="color-entry" onSubmit={e=>{e.preventDefault();apply(draft);}}>
          <span className="color-chip" style={{background:shown}} aria-hidden="true"/>
          <input dir="ltr" aria-label="HEX" maxLength={7} value={draft} onChange={e=>setDraft(e.target.value)}/>
          <ShinyButton className="primary" disabled={!valid(draft)} type="submit">{done}</ShinyButton>
        </form>
      </motion.div>}
    </AnimatePresence>,document.body)}
  </>;
}
