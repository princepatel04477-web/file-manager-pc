import { useEffect, useRef, useState } from 'react';
import { CircleCheckBig, Info, X } from 'lucide-react';
import { motion, useReducedMotion } from 'framer-motion';
import type { CleanResult } from '../../stores/clean-store';
import { readableSize } from '../FileList';

/** Counts up to `target` over ~700 ms; jumps straight there for reduced motion. */
function useCountUp(target: number, reducedMotion: boolean | null): number {
  const [value, setValue] = useState(reducedMotion ? target : 0);
  const frame = useRef<number | null>(null);

  useEffect(() => {
    if (reducedMotion) {
      setValue(target);
      return;
    }
    const duration = 700;
    const started = performance.now();
    const step = (now: number) => {
      const progress = Math.min(1, (now - started) / duration);
      // easeOutCubic, so the number settles instead of stopping dead
      const eased = 1 - Math.pow(1 - progress, 3);
      setValue(target * eased);
      if (progress < 1) frame.current = requestAnimationFrame(step);
    };
    frame.current = requestAnimationFrame(step);
    return () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    };
  }, [target, reducedMotion]);

  return value;
}

interface FreedResultProps {
  result: CleanResult;
  onDismiss: () => void;
}

/** The animated "You freed X" confirmation shown after a cleanup. */
export function FreedResult({ result, onDismiss }: FreedResultProps) {
  const reducedMotion = useReducedMotion();
  const animated = useCountUp(result.bytes, reducedMotion);

  useEffect(() => {
    const timeout = window.setTimeout(onDismiss, 9000);
    return () => window.clearTimeout(timeout);
  }, [onDismiss]);

  return (
    <motion.section
      className="freed-banner"
      role="status"
      aria-live="polite"
      initial={{ opacity: 0, y: reducedMotion ? 0 : -8, scale: reducedMotion ? 1 : 0.985 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: reducedMotion ? 0 : 0.32, ease: [0.22, 1, 0.36, 1] }}
    >
      <span className="freed-icon"><CircleCheckBig size={20} /></span>
      <div className="freed-copy">
        <strong>
          You freed <span className="freed-amount">{readableSize(Math.round(animated))}</span>
        </strong>
        <span>
          {result.items.toLocaleString()} item{result.items === 1 ? '' : 's'} from {result.label}
          {result.label === 'Recycle Bin' ? '' : ' moved to the Recycle Bin'}.
        </span>
      </div>
      {result.detail && <p className="freed-detail"><Info size={14} />{result.detail}</p>}
      <button type="button" className="icon-button" aria-label="Dismiss" onClick={onDismiss}><X size={15} /></button>
    </motion.section>
  );
}
