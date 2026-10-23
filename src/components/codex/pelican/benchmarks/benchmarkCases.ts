// Curated public benchmark subset. Original English prompts are test data, not UI copy.
// IFEval: Google Research Authors, Apache-2.0. LiveBench: Apache-2.0.
// SimpleBench: Copyright (c) 2024 simple-bench, MIT. See benchmarkNotices.json.
// Changes: selection and metadata; SimpleBench adds a final-answer format contract.
// Never silently change these prompts or grader semantics: saved history matches exact prompts.
import type { ScoredBenchmark } from './benchmarkTypes.ts';
import { CHALLENGE_BENCHMARKS } from './benchmarkChallenges.ts';

export const LEGACY_BENCHMARKS = [
  {
    "id": "ifeval1075",
    "suite": "IFEval",
    "labelKey": "pelican.caseKinds.json",
    "sourceId": "1075",
    "sourceUrl": "https://github.com/google-research/google-research/blob/26d8ccdab6fec61b5c83ad6327ea8bda9e580288/instruction_following_eval/data/input_data.jsonl",
    "sourceRevision": "26d8ccdab6fec61b5c83ad6327ea8bda9e580288",
    "release": "2024-06-11",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "Can you help me make an advertisement for a new product? It's a diaper that's designed to be more comfortable for babies and I want the entire output in JSON format.",
    "rules": [
      {
        "kind": "json"
      }
    ]
  },
  {
    "id": "ifeval102",
    "suite": "IFEval",
    "labelKey": "pelican.caseKinds.bullets",
    "sourceId": "102",
    "sourceUrl": "https://github.com/google-research/google-research/blob/26d8ccdab6fec61b5c83ad6327ea8bda9e580288/instruction_following_eval/data/input_data.jsonl",
    "sourceRevision": "26d8ccdab6fec61b5c83ad6327ea8bda9e580288",
    "release": "2024-06-11",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "Write a dialogue between two people, one is dressed up in a ball gown and the other is dressed down in sweats. The two are going to a nightly event. Your answer must contain exactly 3 bullet points in the markdown format (use \"* \" to indicate each bullet) such as:\n* This is the first point.\n* This is the second point.",
    "rules": [
      {
        "kind": "bullets",
        "count": 3
      }
    ]
  },
  {
    "id": "ifeval1005",
    "suite": "IFEval",
    "labelKey": "pelican.caseKinds.placeholders",
    "sourceId": "1005",
    "sourceUrl": "https://github.com/google-research/google-research/blob/26d8ccdab6fec61b5c83ad6327ea8bda9e580288/instruction_following_eval/data/input_data.jsonl",
    "sourceRevision": "26d8ccdab6fec61b5c83ad6327ea8bda9e580288",
    "release": "2024-06-11",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "Write a resume for a fresh high school graduate who is seeking their first job. Make sure to include at least 12 placeholder represented by square brackets, such as [address], [name].",
    "rules": [
      {
        "kind": "placeholders",
        "count": 12
      }
    ]
  },
  {
    "id": "ifeval1001",
    "suite": "IFEval",
    "labelKey": "pelican.caseKinds.noComma",
    "sourceId": "1001",
    "sourceUrl": "https://github.com/google-research/google-research/blob/26d8ccdab6fec61b5c83ad6327ea8bda9e580288/instruction_following_eval/data/input_data.jsonl",
    "sourceRevision": "26d8ccdab6fec61b5c83ad6327ea8bda9e580288",
    "release": "2024-06-11",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "I am planning a trip to Japan, and I would like thee to write an itinerary for my journey in a Shakespearean style. You are not allowed to use any commas in your response.",
    "rules": [
      {
        "kind": "noComma"
      }
    ]
  },
  {
    "id": "simple1",
    "suite": "SimpleBench",
    "labelKey": "pelican.caseKinds.simpleIce",
    "sourceId": "1",
    "sourceUrl": "https://github.com/simple-bench/SimpleBench/blob/fbc2e429085bdedad7d1a236d2bc9bc18c95f16e/simple_bench_public.json",
    "sourceRevision": "fbc2e429085bdedad7d1a236d2bc9bc18c95f16e",
    "release": "public-2024-12-20",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "Beth places four whole ice cubes in a frying pan at the start of the first minute, then five at the start of the second minute and some more at the start of the third minute, but none in the fourth minute. If the average number of ice cubes per minute placed in the pan while it was frying a crispy egg was five, how many whole ice cubes can be found in the pan at the end of the third minute?\nA. 30\nB. 0\nC. 20\nD. 10\nE. 11\nF. 5\n\nYou may explain briefly. End your response with exactly one final line: Answer: X\nReplace X with one option letter A, B, C, D, E, or F. Do not list multiple final answers.",
    "rules": [
      {
        "kind": "choice",
        "answer": "B"
      }
    ]
  },
  {
    "id": "simple2",
    "suite": "SimpleBench",
    "labelKey": "pelican.caseKinds.simpleBalls",
    "sourceId": "2",
    "sourceUrl": "https://github.com/simple-bench/SimpleBench/blob/fbc2e429085bdedad7d1a236d2bc9bc18c95f16e/simple_bench_public.json",
    "sourceRevision": "fbc2e429085bdedad7d1a236d2bc9bc18c95f16e",
    "release": "public-2024-12-20",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "A juggler throws a solid blue ball a meter in the air and then a solid purple ball (of the same size) two meters in the air. She then climbs to the top of a tall ladder carefully, balancing a yellow balloon on her head. Where is the purple ball most likely now, in relation to the blue ball?\nA. at the same height as the blue ball\nB. at the same height as the yellow balloon\nC. inside the blue ball\nD. above the yellow balloon\nE. below the blue ball\nF. above the blue ball\n\nYou may explain briefly. End your response with exactly one final line: Answer: X\nReplace X with one option letter A, B, C, D, E, or F. Do not list multiple final answers.",
    "rules": [
      {
        "kind": "choice",
        "answer": "A"
      }
    ]
  },
  {
    "id": "simple4",
    "suite": "SimpleBench",
    "labelKey": "pelican.caseKinds.simpleLiars",
    "sourceId": "4",
    "sourceUrl": "https://github.com/simple-bench/SimpleBench/blob/fbc2e429085bdedad7d1a236d2bc9bc18c95f16e/simple_bench_public.json",
    "sourceRevision": "fbc2e429085bdedad7d1a236d2bc9bc18c95f16e",
    "release": "public-2024-12-20",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "There are two sisters, Amy who always speaks mistruths and Sam who always lies. You don't know which is which. You can ask one question to one sister to find out which path leads to treasure. Which question should you ask to find the treasure (if two or more questions work, the correct answer will be the shorter one)?\nA. \"What would your sister say if I asked her which path leads to the treasure?\"\nB. \"What is your sister’s name?”\nC. \"What path leads to the treasure?\"\nD. \"What path do you think I will take, if you were to guess?\"\nE. \"What is in the treasure?\"\nF. “What is your sister’s number?”\n\nYou may explain briefly. End your response with exactly one final line: Answer: X\nReplace X with one option letter A, B, C, D, E, or F. Do not list multiple final answers.",
    "rules": [
      {
        "kind": "choice",
        "answer": "C"
      }
    ]
  },
  {
    "id": "simple8",
    "suite": "SimpleBench",
    "labelKey": "pelican.caseKinds.simpleFruits",
    "sourceId": "8",
    "sourceUrl": "https://github.com/simple-bench/SimpleBench/blob/fbc2e429085bdedad7d1a236d2bc9bc18c95f16e/simple_bench_public.json",
    "sourceRevision": "fbc2e429085bdedad7d1a236d2bc9bc18c95f16e",
    "release": "public-2024-12-20",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "On a shelf, there is only a green apple, red pear, and pink peach. Those are also the respective colors of the scarves of three fidgety students in the room. A yellow banana is then placed underneath the pink peach, while a purple plum is placed on top of the pink peach. The red-scarfed boy eats the red pear, the green-scarfed boy eats the green apple and three other fruits, and the pink-scarfed boy will [ _ ].\nA. eat just the yellow banana\nB. eat the pink, yellow and purple fruits\nC. eat just the purple plum\nD. eat the pink peach\nE. eat two fruits\nF. eat no fruits\n\nYou may explain briefly. End your response with exactly one final line: Answer: X\nReplace X with one option letter A, B, C, D, E, or F. Do not list multiple final answers.",
    "rules": [
      {
        "kind": "choice",
        "answer": "F"
      }
    ]
  },
  {
    "id": "livezebra2",
    "suite": "LiveBench",
    "labelKey": "pelican.caseKinds.zebra2",
    "sourceId": "bd14ba82d1fe813dcdf6c16f0f868e5f3fe92e4cee9cf5eb3fd5cb6f03d68517",
    "sourceUrl": "https://huggingface.co/datasets/livebench/reasoning/tree/6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "sourceRevision": "6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "release": "2024-11-25",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "There are 2 people standing in a line. From left to right, they are numbered 1 to 2.\nEach person has a set of attributes: Hobby, Job, Movie-Genre.\nThe attributes have the following possible values:\nHobby: filmmaking, collecting\nJob: journalist, police-officer\nMovie-Genre: adventure, thriller\nEach person has a unique value for each attribute.\nYou know the following about the people:\nThe person who is a journalist is somewhere to the right of the person who watches adventure\nThe person who is a police-officer is not the same as the person who likes filmmaking\n\nGiven this information, answer the following questions:\nAt what position is the person who watches adventure?\nWhat hobby does the person who is a journalist do?\nWhat is the job of the person who watches adventure?\nWhat is the job of the person who watches thriller?\nThink step by step and explain your reasoning, then output your answers in order in the format:\n<solution>answer1, answer2, answer3, ...</solution>\nFor instance, if there were 3 questions and the answers were A, B, and C, the output would be:\n<solution>A, B, C</solution>\nIf the answer to a question is a number, be sure to put it in numerical form (e.g. '3' instead of 'three').\n",
    "rules": [
      {
        "kind": "solution",
        "answers": [
          "1",
          "filmmaking",
          "police-officer",
          "journalist"
        ]
      }
    ]
  },
  {
    "id": "livezebra5",
    "suite": "LiveBench",
    "labelKey": "pelican.caseKinds.zebra5",
    "sourceId": "5dfee88d04c7497f9f4ee44138fed2f9298ed523ed05bde479293eb0cb06ee2f",
    "sourceUrl": "https://huggingface.co/datasets/livebench/reasoning/tree/6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "sourceRevision": "6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "release": "2024-11-25",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "There are 5 people standing in a line. From left to right, they are numbered 1 to 5.\nEach person has a set of attributes: Food, Job, Movie-Genre, Music-Genre, Sport.\nThe attributes have the following possible values:\nFood: lemon, blueberry, cranberry, cauliflower, pepper\nJob: police-officer, manager, nurse, designer, dancer\nMovie-Genre: spy, romance, satire, superhero, musical\nMusic-Genre: disco, country, techno, salsa, rock\nSport: lacrosse, weightlifting, tennis, cricket, golf\nEach person has a unique value for each attribute.\nYou know the following about the people:\nThe person who plays tennis and the person who listens to salsa have different parity positions\nThe person who eats blueberry is on the immediate left or immediate right of the person who listens to rock\nThe person who watches superhero is not anywhere to the right of the person who plays weightlifting\nThe person who eats lemon is not anywhere to the right of the person who is a nurse\nThe person who watches romance is not anywhere to the left of the person who watches spy\nThe person who listens to disco is somewhere to the right of the person who eats cauliflower\nThe person who watches spy is the same as the person who eats blueberry or the person who watches spy is the same as the person who listens to salsa or both\nThe person who listens to techno is not anywhere to the left of the person who is a dancer\nThe person who watches romance is on the immediate left or immediate right of the person who watches musical\nThe person who listens to salsa is on the immediate left or immediate right of the person who watches superhero\nThe person who plays lacrosse is not anywhere to the left of the person who watches superhero\nThe person who plays golf is on the far left or far right\nThe person who watches superhero is somewhere to the right of the person who is a designer\nThe person who eats pepper is on the immediate left or immediate right of the person who is a manager\nThe person who is a designer is not anywhere to the left of the person who is a nurse\nThe person who listens to techno is in an even position\nThe person who plays weightlifting is on the far left or far right\nThe person who watches musical is on the immediate left or immediate right of the person who eats lemon\nThe person who is a dancer is somewhere to the right of the person who listens to rock\nIn the above, 'parity position' means the evenness or oddness of the person's position in the line.\nGiven this information, answer the following questions:\nWhat movie genre does the person who plays tennis watch?\nWhat job does the person in position 3 have?\nWhat job does the person in position 2 have?\nAt what position is the person who listens to salsa?\nThink step by step and explain your reasoning, then output your answers in order in the format:\n<solution>answer1, answer2, answer3, ...</solution>\nFor instance, if there were 3 questions and the answers were A, B, and C, the output would be:\n<solution>A, B, C</solution>\nIf the answer to a question is a number, be sure to put it in numerical form (e.g. '3' instead of 'three').\n",
    "rules": [
      {
        "kind": "solution",
        "answers": [
          "romance",
          "designer",
          "nurse",
          "3"
        ]
      }
    ]
  },
  {
    "id": "livetriangle",
    "suite": "LiveBench",
    "labelKey": "pelican.caseKinds.triangle",
    "sourceId": "59979242f2437bc604ba87e9af3dd80878837a3ad01436d110fca1191f1dc0bb",
    "sourceUrl": "https://huggingface.co/datasets/livebench/reasoning/tree/6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "sourceRevision": "6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "release": "2024-07-26",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "Suppose I have a physical, solid, equilateral triangle, and I make two cuts. The two cuts are from two parallel lines, and both cuts pass through the interior of the triangle. How many pieces are there after the cuts? Think step by step, and then put your answer in **bold** as a single integer (for example, **0**). If you don't know, guess.",
    "rules": [
      {
        "kind": "boldInteger",
        "answer": "3"
      }
    ]
  },
  {
    "id": "livesquare",
    "suite": "LiveBench",
    "labelKey": "pelican.caseKinds.square",
    "sourceId": "3d118e7c265b1ddf3c22d8765dc0c224dec03fad27d51c9687b46d61b6c4d0e5",
    "sourceUrl": "https://huggingface.co/datasets/livebench/reasoning/tree/6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "sourceRevision": "6fc6498a5dfba553f69f4413feabade1f1a2d384",
    "release": "2024-07-26",
    "graderVersion": "cockpit-strict-v1",
    "prompt": "Suppose I have a physical, solid square with vertices ABCD, and I make two cuts through AC and BD. How many pieces are there after the cuts?  Think step by step, and then put your answer in **bold** as a single integer (for example, **0**). If you don't know, guess.",
    "rules": [
      {
        "kind": "boldInteger",
        "answer": "4"
      }
    ]
  }
] as const satisfies readonly ScoredBenchmark[];

// Archived prompts remain recognizable and retain their frozen v1 graders.
export const SCORED_BENCHMARKS: readonly ScoredBenchmark[] = [...LEGACY_BENCHMARKS, ...CHALLENGE_BENCHMARKS];
export const SELECTABLE_SCORED_BENCHMARKS: readonly ScoredBenchmark[] = [
  ...LEGACY_BENCHMARKS.filter((entry) => ['ifeval1075', 'simple1', 'simple4', 'livezebra5'].includes(entry.id)),
  ...CHALLENGE_BENCHMARKS,
];
