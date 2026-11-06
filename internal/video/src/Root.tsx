import React from 'react';
import {Composition} from 'remotion';
import {Intro, TOTAL} from './Video';
import {IntroV2, TOTAL_V2} from './V2';

export const Root: React.FC = () => (
  <>
    <Composition id="Intro" component={Intro} durationInFrames={TOTAL} fps={30} width={1920} height={1080} />
    <Composition id="IntroV2" component={IntroV2} durationInFrames={TOTAL_V2} fps={30} width={1920} height={1080} />
  </>
);
