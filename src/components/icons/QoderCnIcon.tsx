import { CSSProperties } from 'react';
import qoderIcon from '../../assets/icons/qoder.png';

type QoderCnIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function QoderCnIcon({ className = 'nav-item-icon', style }: QoderCnIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={qoderIcon}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
