import React from 'react';
import { Tabs, TabsProps } from 'antd';
import classNames from 'classnames';
import './style.scss';

const IndicatorTabs: React.FC<TabsProps> = ({ className, ...restProps }) => {
  return <Tabs className={classNames('indicator-tabs', className)} {...restProps} />;
};

export default IndicatorTabs;
