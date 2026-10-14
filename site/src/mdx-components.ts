import Diagram from './components/Diagram.astro';
import TestCounts from './components/TestCounts.astro';

// Components for docs/ MDX, which cannot import anything itself.
export const components = { Diagram, TestCounts };
