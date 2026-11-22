# Drawing react-ui programs in OpenUINative

OpenUINative draws 25 react-ui components directly. Eight more are parts of
native controls. Table consumes `Col`, line and bar charts consume `Series`,
and the tab and selection controls consume `TabItem`, `SelectItem`,
`CheckBoxItem`, `RadioItem`, `ChipItem` and `OptionCard`. A part drawn on its
own gets the default placeholder.

`Table([Col(label, data), …])` draws rows up to the longest column, ten rows
per page. A short column leaves empty cells. A cell containing an element
draws at its original place in the tree, including its buttons and actions.
`PieChart(labels, values)` draws a sector for each paired value and supports
the donut variant. The native chart uses a circular plot.

The bench library's `Table(columns, rows)` and `PieChart(slices)` also draw.
Bench tables retain their action buttons for strings in an action column.
React-ui tables show those strings as text, as upstream does.

Of the 152 recorded react-ui programs, 18 draw with no placeholder anywhere
in their evaluated trees, up from 12. The replay includes elements in tabs
and table pages that are initially out of view.

## Components drawn as placeholders

By default each of these 49 components shows a placeholder bearing its name.
An app can supply its own view through the component drawing hook.

- Layout and navigation — `Accordion`, `AccordionItem`, `Carousel`,
  `InlineHeader`, `Modal`, `Separator`, `Steps`, `StepsItem`.
- Text and media — `BoldText`, `CodeBlock`, `Icon`, `IconText`, `Image`,
  `ImageBlock`, `ImageGallery`, `ImageText`, `ImageTextLarge`, `Label`,
  `MarkDownRenderer`, `Text`.
- Charts and metrics — `AreaChart`, `HorizontalBarChart`,
  `MetricIndicatorInline`, `MetricIndicatorWithStrikethrough`, `Point`,
  `RadarChart`, `RadialChart`, `ScatterChart`, `ScatterSeries`,
  `SingleStackedBarChart`, `Slice`.
- Cards and lists — `CompositeCardBlock`, `CompositeCardItem`,
  `ContextCardBlock`, `ContextCardItem`, `EntityList`, `ListBlock`, `ListItem`,
  `OverviewCardBlock`, `OverviewCardItem`, `SnippetCardBlock`, `SnippetCardItem`,
  `TagBlock`, `VisualCardBlock`, `VisualCardItem`.
- Controls — `EditableTable`, `IconButton`, `SwitchGroup`, `SwitchItem`.
