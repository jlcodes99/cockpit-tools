import { CSSProperties } from 'react';
import qoderIcon from '../../assets/icons/qoder.png';
import qoderAppIcon from '../../assets/icons/qoder-app.png';
import qoderCnAppIcon from '../../assets/icons/qoder-cn-app.png';

type QoderIconProps = {
  className?: string;
  style?: CSSProperties;
};

type QoderImageIconProps = QoderIconProps & {
  src: string;
};

function QoderImageIcon({ src, className = 'nav-item-icon', style }: QoderImageIconProps) {
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

export function QoderIcon(props: QoderIconProps) {
  return <QoderImageIcon {...props} src={qoderIcon} />;
}

export function QoderAppIcon(props: QoderIconProps) {
  return <QoderImageIcon {...props} src={qoderAppIcon} />;
}

export function QoderCnIdeIcon(props: QoderIconProps) {
  return <QoderImageIcon {...props} src={qoderIcon} />;
}

export function QoderCnAppIcon(props: QoderIconProps) {
  return <QoderImageIcon {...props} src={qoderCnAppIcon} />;
}
