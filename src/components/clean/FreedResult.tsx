import { useEffect, useRef, useState } from 'react';
import { CircleCheckBig, Info, X } from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
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

/** A few sparks that drift once and stop, so the banner celebrates without looping. */
function Sparks({ running }: { running: boolean }) {
  const reducedMotion = useReducedMotion();
  if (reducedMotion || !running) return null;
  return (
    <span className="freed-sparks" aria-hidden="true">
      {[
        { x: -26, y: -22, delay: 0.06 },
        { x: 30, y: -26, delay: 0.1 },
        { x: -34, y: 12, delay: 0.14 },
        { x: 38, y: 8, delay: 0.02 },
      ].map((spark, index) => (
        <motion.i
          key={index}
          initial={{ opacity: 0, scale: 0.4, x: 0, y: 0 }}
          animate={{ opacity: [0, 1, 0], scale: [0.4, 1.1, 0.5], x: spark.x, y: spark.y }}
          transition={{ duration: 0.85, delay: spark.delay, ease: 'easeOut' }}
        />
      ))}
    </span>
  );
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
    <AnimatePresence>
      <motion.section
        key={result.label + result.bytes}
        className="freed-banner"
        role="status"
        aria-live="polite"
        initial={{ opacity: 0, y: reducedMotion ? 0 : -10, scale: reducedMotion ? 1 : 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: reducedMotion ? 0 : -6, scale: reducedMotion ? 1 : 0.99 }}
        transition={{ duration: reducedMotion ? 0 : 0.34, ease: [0.22, 1, 0.36, 1] }}
      >
        <span className="freed-icon">
          <motion.span
            className="freed-icon-inner"
            initial={reducedMotion ? false : { scale: 0.5, rotate: -18 }}
            animate={{ scale: 1, rotate: 0 }}
            transition={{ type: 'spring', stiffness: 380, damping: 16, delay: reducedMotion ? 0 : 0.08 }}
          >
            <CircleCheckBig size={20} />
          </motion.span>
          <Sparks running={result.bytes > 0} />
        </span>
        <div className="freed-copy">
          <strong>
            You freed <motion.span className="freed-amount" layout={!reducedMotion}>{readableSize(Math.round(animated))}</motion.span>
          </strong>
          <span>
            {result.items.toLocaleString()} item{result.items === 1 ? '' : 's'} from {result.label}
            {result.label === 'Recycle Bin' ? '' : ' moved to the Recycle Bin'}.
          </span>
        </div>
        {result.detail && <p className="freed-detail"><Info size={14} />{result.detail}</p>}
        <button type="button" className="icon-button" aria-label="Dismiss" onClick={onDismiss}><X size={15} /></button>
      </motion.section>
    </AnimatePresence>
  );
}
