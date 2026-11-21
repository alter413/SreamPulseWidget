import React, {useEffect, useId, useRef, useState} from 'react';
import {createPortal} from 'react-dom';
import {AnimatePresence, motion, useReducedMotion} from 'motion/react';

export function placePopup(button, preferredHeight, minWidth=160) {
  const box = button.getBoundingClientRect();
  if (box.bottom < 0 || box.top > window.innerHeight) return null;
  const below = window.innerHeight - box.bottom - 13;
  const above = box.top - 13;
  const up = below < preferredHeight && above > below;
  const width = Math.min(window.innerWidth - 16, Math.max(minWidth, box.width));
  const left = Math.min(window.innerWidth - width - 8, Math.max(8,
    getComputedStyle(button).direction === 'rtl' ? box.left : box.right - width));
  return {left, width, up, maxHeight: Math.min(312, up ? above : below),
    top: up ? undefined : box.bottom + 6, bottom: up ? window.innerHeight - box.top + 6 : undefined};
}

export default function ChoiceMenu({id, label, value, options, onChange}) {
  const generatedId = useId();
  const listId = generatedId + '-list';
  const trigger = useRef(null);
  const list = useRef(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [position, setPosition] = useState(null);
  const reduced = useReducedMotion();
  const selected = Math.max(0, options.findIndex(([key]) => key === value));

  function measure() {
    const button = trigger.current;
    if (!button) return;
    const next = placePopup(button, Math.min(220, options.length * 38 + 10));
    if(next) setPosition(next);
    else setOpen(false);
  }

  function show(index = selected) {
    measure();
    setActive(index);
    setOpen(true);
  }
  function select(index) {
    const item = options[index];
    if (item) onChange(item[0]);
    setOpen(false);
    trigger.current?.focus();
  }

  useEffect(() => {
    if (!open) return;
    function outside(e) {
      if (!trigger.current?.contains(e.target) && !list.current?.contains(e.target)) setOpen(false);
    }
    const blur = () => setOpen(false);
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('blur', blur);
    window.addEventListener('resize', measure);
    window.addEventListener('scroll', measure, true);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('blur', blur);
      window.removeEventListener('resize', measure);
      window.removeEventListener('scroll', measure, true);
    };
  }, [open, options.length]);

  useEffect(() => {
    if (!open || !list.current) return;
    const item = list.current.children[active];
    if (!item) return;
    const bounds = list.current.getBoundingClientRect();
    const itemBounds = item.getBoundingClientRect();
    if (itemBounds.top < bounds.top + 4) list.current.scrollTop -= bounds.top - itemBounds.top + 4;
    else if (itemBounds.bottom > bounds.bottom - 4) list.current.scrollTop += itemBounds.bottom - bounds.bottom + 4;
  }, [open, active]);

  function keys(e) {
    if (e.key === 'Escape' && open) {
      e.preventDefault();
      setOpen(false);
    } else if (e.key === 'Tab') {
      setOpen(false);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const delta = e.key === 'ArrowDown' ? 1 : -1;
      if (!open) show();
      else setActive((active + delta + options.length) % options.length);
    } else if (open && (e.key === 'Home' || e.key === 'End')) {
      e.preventDefault();
      setActive(e.key === 'Home' ? 0 : options.length - 1);
    } else if (open && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      select(active);
    } else if (open && e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
      const search = e.key.toLocaleLowerCase();
      const next = options.findIndex(([, text], index) => index > active && text.toLocaleLowerCase().startsWith(search));
      const first = next >= 0 ? next : options.findIndex(([, text]) => text.toLocaleLowerCase().startsWith(search));
      if (first >= 0) setActive(first);
    }
  }

  const panel = <AnimatePresence>
    {open && position && <motion.div key="list" className="choice-list" role="listbox" id={listId} aria-label={label} ref={list}
      style={{left:position.left, width:position.width, top:position.top, bottom:position.bottom,
        maxHeight:position.maxHeight, transformOrigin:position.up?'bottom center':'top center'}}
      initial={reduced?false:{opacity:0,scale:.97,y:position.up?4:-4}}
      animate={{opacity:1,scale:1,y:0}}
      exit={reduced?{opacity:0}:{opacity:0,scale:.98,y:position.up?3:-3}}
      transition={{duration:reduced?0:.17,ease:[.2,.72,.35,1]}}>
      {options.map(([key, text], index) => <div key={key} id={listId+'-'+index} role="option" aria-selected={index===selected}
        data-active={index===active} className="choice-option" onPointerMove={()=>setActive(index)}
        onPointerDown={e=>e.preventDefault()} onClick={()=>select(index)}>
        <span>{text}</span>
        {index===selected && <span aria-hidden="true" className="choice-check">✓</span>}
      </div>)}
    </motion.div>}
  </AnimatePresence>;

  return <>
    <button ref={trigger} type="button" className="choice-trigger" id={id||generatedId}
      role="combobox" aria-label={label} aria-expanded={open} aria-controls={open?listId:undefined}
      aria-activedescendant={open?listId+'-'+active:undefined} onKeyDown={keys}
      onClick={()=>open?setOpen(false):show()}>
      <span>{options[selected]?.[1]}</span>
      <svg className="choice-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4 6 4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/></svg>
    </button>
    {createPortal(panel, document.body)}
  </>;
}
