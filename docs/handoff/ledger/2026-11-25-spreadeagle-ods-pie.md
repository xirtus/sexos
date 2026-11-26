# Spreadeagle ODS pie charts

Spreadeagle now maps plain one-series native pie charts to ODF `chart:circle` objects and restores them on import. The chart retains its live values, category labels, title, legend and sheet position. Unsupported pie rotation, exploded slices, multiple series and advanced design settings refuse ODS export or chart import instead of silently changing the chart; XLSX remains the path for those charts.

`apps/spreadeagle/assets/lo-pie.ods` is a LibreOffice re-saved chart fixture. The focused host tests check native ODS round trip and reopening the fixture. The Spreadeagle QEMU gate opens that fixture, checks that its imported chart kinds are Pie then Line, and captures `spreadeagle-ods-pie.ppm` with the pie in the visible first row. Full build and guest evidence run on GitHub CI.
