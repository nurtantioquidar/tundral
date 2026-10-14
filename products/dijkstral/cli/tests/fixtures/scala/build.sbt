lazy val root = project
  .in(file("."))
  .settings(
    name := "shop",
    scalaVersion := "3.3.3",
    libraryDependencies += "org.scalameta" %% "munit" % "1.0.0" % Test
  )
