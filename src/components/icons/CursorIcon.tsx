import { CSSProperties } from 'react';
import cursorIcon from '../../assets/icons/cursor-menu.png';

type CursorIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function CursorIcon({ className = 'nav-item-icon', style }: CursorIconProps) {
  return <img src={cursorIcon} className={className} style={style} alt="" aria-hidden="true" />;
}
