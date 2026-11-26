// Shiny Button and Avatar Circles are adapted from Magic UI (MIT).
import React, {forwardRef} from 'react';
import {motion} from 'motion/react';

export const ShinyButton = forwardRef(function ShinyButton({className='', type='button', ...rest}, ref) {
  return <motion.button ref={ref} type={type} className={'shiny-button '+className}
    whileTap={rest.disabled?undefined:{scale:.97}} {...rest}/>;
});

export function ThemeToggler({theme, onThemeChange, label}) {
  const light = theme === 'light';
  return <ShinyButton className="theme-button" onClick={()=>onThemeChange(light?'dark':'light')} title={label} aria-label={label}>
    <i className={light?'fi fi-rc-moon-stars':'fi fi-br-brightness'} aria-hidden="true"/>
  </ShinyButton>;
}

export function AvatarCircles({people, limit=5}) {
  const shown=people.slice(0,limit);
  const extra=people.slice(limit);
  return <div className="avatar-circles" aria-label={people.map(person=>person.name).join(', ')}>
    {shown.map((person,index)=><span className="avatar-circle" key={index} title={person.name}>
      <span aria-hidden="true">{Array.from(person.name)[0]?.toLocaleUpperCase()}</span>
      {person.avatar && <img src={person.avatar} alt="" loading="lazy" referrerPolicy="no-referrer"
        onError={e=>{e.currentTarget.hidden=true;}}/>}
    </span>)}
    {extra.length>0 && <span className="avatar-circle" title={extra.map(person=>person.name).join(', ')}>+{extra.length}</span>}
  </div>;
}
