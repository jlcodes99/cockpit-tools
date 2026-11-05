import { CSSProperties } from 'react';
import minimaxCodeIcon from '../../assets/icons/minimax-code.png';

type MinimaxCodeIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function MinimaxCodeIcon({ className = 'nav-item-icon', style }: MinimaxCodeIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={minimaxCodeIcon}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
