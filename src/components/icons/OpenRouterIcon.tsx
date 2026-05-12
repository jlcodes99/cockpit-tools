import { CSSProperties } from 'react';
import openrouterIcon from '../../assets/icons/openrouter.png';

type OpenRouterIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function OpenRouterIcon({ className = 'nav-item-icon', style }: OpenRouterIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={openrouterIcon}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
