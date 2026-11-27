import { CSSProperties } from 'react';
import kiroIcon from '../../assets/icons/kiro-menu.png';

type KiroIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function KiroIcon({ className = 'nav-item-icon', style }: KiroIconProps) {
  return <img src={kiroIcon} className={className} style={style} alt="" aria-hidden="true" />;
}
