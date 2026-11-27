import { CSSProperties } from 'react';
import traeWorkIcon from '../../assets/icons/trae-work.png';

type TraeIconProps = {
  className?: string;
  style?: CSSProperties;
};

type TraeImageIconProps = TraeIconProps & {
  src: string;
};

function TraeImageIcon({ src, className = 'nav-item-icon', style }: TraeImageIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={src}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}

export function TraeIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeWorkIcon} />;
}

export function TraeSoloIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeWorkIcon} />;
}

export function TraeCnIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeWorkIcon} />;
}

export function TraeSoloCnIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeWorkIcon} />;
}
