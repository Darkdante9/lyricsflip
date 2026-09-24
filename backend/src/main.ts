import { NestFactory } from '@nestjs/core';
import { AppModule } from './app.module';
import { DocumentBuilder, SwaggerModule } from '@nestjs/swagger';
import { swaggerConfig, swaggerCustomOptions } from './utility/Swagger';
import { SocketIOAdapter } from './config/socket-io.config';
import { Logger, ValidationPipe } from '@nestjs/common';
import { CustomLoggerService } from './logger/custom-logger.service';
import { LoggingInterceptor } from './interceptors/logging.interceptor';
import { ConfigService } from '@nestjs/config';

async function bootstrap() {
  const app = await NestFactory.create(AppModule, {
    logger: new CustomLoggerService(),
  });

  const config = app.get(ConfigService);
  const logger = new Logger('Main');

  app.useGlobalPipes(
    new ValidationPipe({
      whitelist: true,
      transform: true,
      forbidNonWhitelisted: true,
    }),
  );

  app.enableCors({
    origin: config.get<string>('cors.origin') ?? '*',
    credentials: true,
  });

  app.useWebSocketAdapter(new SocketIOAdapter(app));

  app.useGlobalInterceptors(new LoggingInterceptor());

  if (process.env.NODE_ENV !== 'production') {
    const document = SwaggerModule.createDocument(app, swaggerConfig);
    SwaggerModule.setup('api/docs', app, document, swaggerCustomOptions);
  }

  const port = config.get<number>('port') ?? process.env.PORT ?? 3000;
  await app.listen(port);
  logger.log(`Application listening on port ${port}`);
}

bootstrap();
