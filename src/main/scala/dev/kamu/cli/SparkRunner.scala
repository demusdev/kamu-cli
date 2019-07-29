package dev.kamu.cli

import java.io.OutputStream
import java.util.zip.{ZipEntry, ZipOutputStream}

import org.apache.hadoop.fs.{FileSystem, Path}
import dev.kamu.core.manifests.utils.fs._
import org.apache.commons.io.IOUtils
import org.apache.log4j.{Level, LogManager, Logger}

abstract class SparkRunner(
  fileSystem: FileSystem,
  logLevel: Level
) {
  protected var logger: Logger = LogManager.getLogger(getClass.getName)

  def submit(
    repo: RepositoryVolumeMap,
    appClass: String,
    extraFiles: Map[String, OutputStream => Unit] = Map.empty,
    jars: Seq[Path] = Seq.empty
  ): Unit = {
    val tmpJar =
      if (extraFiles.nonEmpty)
        prepareJar(extraFiles)
      else
        null

    val loggingConfig = prepareLog4jConfig()

    try {
      submit(repo, appClass, Seq(tmpJar) ++ jars, loggingConfig)
    } finally {
      if (tmpJar != null)
        fileSystem.delete(tmpJar, false)

      fileSystem.delete(loggingConfig, false)
    }
  }

  protected def submit(
    repo: RepositoryVolumeMap,
    appClass: String,
    jars: Seq[Path],
    loggingConfig: Path
  )

  protected def assemblyPath: Path = {
    new Path(getClass.getProtectionDomain.getCodeSource.getLocation.toURI)
  }

  protected def prepareJar(files: Map[String, OutputStream => Unit]): Path = {
    val tmpDir = new Path(sys.props("java.io.tmpdir"))
    val jarPath = tmpDir.resolve("kamu-configs.jar")

    logger.debug(s"Writing temporary JAR to: $jarPath")

    val fileStream = fileSystem.create(jarPath, true)
    val zipStream = new ZipOutputStream(fileStream)

    files.foreach {
      case (name, writeFun) =>
        zipStream.putNextEntry(new ZipEntry(name))
        writeFun(zipStream)
        zipStream.closeEntry()
    }

    zipStream.close()
    jarPath
  }

  protected def prepareLog4jConfig(): Path = {
    val tmpDir = new Path(sys.props("java.io.tmpdir"))
    val path = tmpDir.resolve("kamu-spark-log4j.properties")

    val resourceName = logLevel match {
      case Level.ALL | Level.TRACE | Level.DEBUG | Level.INFO =>
        "spark.info.log4j.properties"
      case Level.WARN | Level.ERROR =>
        "spark.warn.log4j.properties"
      case _ =>
        "spark.info.log4j.properties"
    }

    val configStream = getClass.getClassLoader.getResourceAsStream(resourceName)
    val outputStream = fileSystem.create(path, true)

    IOUtils.copy(configStream, outputStream)

    outputStream.close()
    configStream.close()

    path
  }
}
