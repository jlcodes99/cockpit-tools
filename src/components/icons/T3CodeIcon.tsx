import type { CSSProperties } from 'react';
import iconUrl from '../../assets/icons/t3code.png';

interface T3CodeIconProps {
  size?: number;
  className?: string;
  style?: CSSProperties;
}

export function T3CodeIcon({ size = 20, className, style }: T3CodeIconProps) {
  return (
    <img
      src={iconUrl}
      alt=""
      aria-hidden="true"
      className={className}
      style={{
        width: size,
        height: size,
        objectFit: 'contain',
        borderRadius: Math.max(3, Math.round(size * 0.22)),
        flexShrink: 0,
        ...style,
      }}
    />
  );
}
