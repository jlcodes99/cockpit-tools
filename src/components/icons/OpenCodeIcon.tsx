import { CSSProperties } from 'react';
import opencodeIcon from '../../assets/icons/opencode.svg';

type OpenCodeIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function OpenCodeIcon({ className = 'nav-item-icon', style }: OpenCodeIconProps) {
  return <img src={opencodeIcon} className={className} style={style} alt="" aria-hidden="true" />;
}
