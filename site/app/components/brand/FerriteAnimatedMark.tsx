import { useEffect, useState } from 'react';
import { FerriteMark } from './FerriteMark';
let played = false;
/** One CSS sequence per client session; no timers, RAF or visibility observers. */
export function FerriteAnimatedMark() {
  const [animate, setAnimate] = useState(false);
  useEffect(() => {
    if (!played) { played = true; setAnimate(true); }
  }, []);
  return <FerriteMark decorative size={20} className={animate ? 'ferrite-mark--animated' : ''} />;
}
